#!/usr/bin/env python3
"""Convert an EfficientAT MobileNetV3 checkpoint (PyTorch, AudioSet) into a Reis
neural model file (.rnm) + a reference test vector.

    cd EfficientAT && python export_efficientat.py mn10_as OUT_DIR

The .rnm format (read by core/src/nn/model.rs) is deliberately framework-free:
  b"RNNM" | u32 version=1 | u32 header_len | header (UTF-8 JSON) | pad to 16 | tensor data
The JSON header describes
  * the audio front-end (sample rate, STFT, mel matrix, log/normalisation),
  * a flat list of ops (conv / se / push / add / gap / linear / output) that the
    Rust interpreter executes in order, BatchNorm already folded into convs,
  * the tensors (offset, shape, dtype f16|f32) and the class labels.
Any other CNN can be exported to the same op set without touching Rust.

The reference file (<name>.ref.bin) holds a fixed test waveform and the PyTorch
outputs (mel, embedding, logits) so the Rust implementation is verified
numerically (`cargo test` / `reis model-check`).
"""
import csv
import json
import os
import struct
import sys

import numpy as np
import torch
import torch.nn as nn
import torchvision.ops.misc as tvm

# EfficientAT targets the old torchvision name of the same Sequential(conv, norm, act).
if not hasattr(tvm, "ConvNormActivation"):
    tvm.ConvNormActivation = tvm.Conv2dNormActivation

sys.path.insert(0, os.getcwd())
from models.mn.model import get_model  # noqa: E402
from models.mn.block_types import InvertedResidual, ConcurrentSEBlock  # noqa: E402
from helpers.utils import NAME_TO_WIDTH  # noqa: E402

SR, N_FFT, HOP, WIN, N_MELS = 32000, 1024, 320, 800, 128
FMIN, FMAX = 0.0, SR // 2 - 1000


class Writer:
    def __init__(self):
        self.tensors = {}
        self.blob = bytearray()

    def add(self, name, arr, dtype="f16"):
        arr = np.ascontiguousarray(arr)
        while len(self.blob) % 16:
            self.blob.append(0)
        data = arr.astype(np.float16 if dtype == "f16" else np.float32).tobytes()
        self.tensors[name] = {"offset": len(self.blob), "shape": list(arr.shape), "dtype": dtype}
        self.blob += data
        return name


def fold_conv(seq):
    """Sequential(conv, bn[, act]) -> (W, b, act) with BN folded."""
    conv, bn = seq[0], seq[1]
    act = "none"
    if len(seq) > 2:
        act = {"Hardswish": "hswish", "ReLU": "relu", "ReLU6": "relu6", "SiLU": "silu"}[type(seq[2]).__name__]
    w = conv.weight.detach().double().numpy()
    b = conv.bias.detach().double().numpy() if conv.bias is not None else np.zeros(w.shape[0])
    s = bn.weight.detach().double().numpy() / np.sqrt(bn.running_var.detach().double().numpy() + bn.eps)
    w = w * s[:, None, None, None]
    b = (b - bn.running_mean.detach().double().numpy()) * s + bn.bias.detach().double().numpy()
    assert conv.dilation == (1, 1), "dilation not supported"
    return w, b, act, conv


def conv_op(wr, name, seq):
    w, b, act, conv = fold_conv(seq)
    return {"op": "conv", "w": wr.add(name + ".w", w), "b": wr.add(name + ".b", b, "f32"),
            "cin": conv.in_channels, "cout": conv.out_channels, "k": list(conv.kernel_size),
            "stride": list(conv.stride), "pad": list(conv.padding), "groups": conv.groups, "act": act}


def export_net(model, wr):
    ops = []
    for i, layer in enumerate(model.features):
        if isinstance(layer, InvertedResidual):
            if layer.use_res_connect:
                ops.append({"op": "push"})
            for j, sub in enumerate(layer.block):
                nm = f"f{i}.{j}"
                if isinstance(sub, ConcurrentSEBlock):
                    assert len(sub.conc_se_layers) == 1, "only channel SE supported"
                    se = sub.conc_se_layers[0]
                    assert se.se_dim == [2, 3], "only channel SE supported"
                    ops.append({"op": "se",
                                "w1": wr.add(nm + ".w1", se.fc1.weight.detach().numpy()),
                                "b1": wr.add(nm + ".b1", se.fc1.bias.detach().numpy(), "f32"),
                                "w2": wr.add(nm + ".w2", se.fc2.weight.detach().numpy()),
                                "b2": wr.add(nm + ".b2", se.fc2.bias.detach().numpy(), "f32"),
                                "act": "relu", "gate": "sigmoid"})
                else:
                    ops.append(conv_op(wr, nm, sub))
            if layer.use_res_connect:
                ops.append({"op": "add"})
        else:
            ops.append(conv_op(wr, f"f{i}", layer))
    ops.append({"op": "gap"})
    ops.append({"op": "output", "name": "embedding"})
    cl = model.classifier  # AdaptiveAvgPool, Flatten, Linear, Hardswish, Dropout, Linear
    ops.append({"op": "linear", "w": wr.add("c1.w", cl[2].weight.detach().numpy()),
                "b": wr.add("c1.b", cl[2].bias.detach().numpy(), "f32"), "act": "hswish"})
    ops.append({"op": "linear", "w": wr.add("c2.w", cl[5].weight.detach().numpy()),
                "b": wr.add("c2.b", cl[5].bias.detach().numpy(), "f32"), "act": "none"})
    ops.append({"op": "output", "name": "logits", "post": "sigmoid"})
    return ops


def mel_matrix():
    import torchaudio
    mb, _ = torchaudio.compliance.kaldi.get_mel_banks(N_MELS, N_FFT, SR, FMIN, FMAX,
                                                       vtln_low=100.0, vtln_high=-500., vtln_warp_factor=1.0)
    return torch.nn.functional.pad(mb, (0, 1), mode="constant", value=0).numpy()  # (128, 513)


def window():
    w = torch.hann_window(WIN, periodic=False).numpy()
    full = np.zeros(N_FFT, np.float32)
    left = (N_FFT - WIN) // 2
    full[left:left + WIN] = w
    return full


def frontend_np(x, mel, win):
    """Reference front-end in numpy, identical to EfficientAT's AugmentMelSTFT (eval)."""
    x = x[1:] - 0.97 * x[:-1]  # conv1d with [-0.97, 1]
    xp = np.pad(x, (N_FFT // 2, N_FFT // 2), mode="reflect")
    n = 1 + (len(xp) - N_FFT) // HOP
    frames = np.stack([xp[i * HOP:i * HOP + N_FFT] * win for i in range(n)])
    p = np.abs(np.fft.rfft(frames, axis=1)) ** 2  # (T, 513)
    m = mel @ p.T  # (128, T)
    return ((np.log(m + 1e-5) + 4.5) / 5.0).astype(np.float32)


def labels():
    with open("metadata/class_labels_indices.csv") as f:
        rows = list(csv.reader(f))[1:]
    return [r[2] for r in rows]


def main():
    name, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    model = get_model(width_mult=NAME_TO_WIDTH(name), pretrained_name=name).eval()
    wr = Writer()
    mel = mel_matrix()
    win = window()
    ops = export_net(model, wr)
    header = {
        "format": "reis-nn", "version": 1, "name": name,
        "source": "EfficientAT (Schmid et al., ICASSP 2023), https://github.com/fschmid56/EfficientAT, MIT licence",
        "trained_on": "AudioSet (527 classes)",
        "frontend": {"type": "logmel", "sample_rate": SR, "n_fft": N_FFT, "hop": HOP, "preemphasis": 0.97,
                     "center": "reflect", "window": wr.add("frontend.window", win, "f32"),
                     "mel": wr.add("frontend.mel", mel, "f32"), "log_eps": 1e-5, "norm_add": 4.5, "norm_div": 5.0},
        "input": {"layout": "1 x mels x frames"},
        "ops": ops,
        "labels": labels(),
    }
    header["tensors"] = wr.tensors
    hj = json.dumps(header, ensure_ascii=False).encode()
    with open(f"{out}/{name}.rnm", "wb") as f:
        f.write(b"RNNM" + struct.pack("<II", 1, len(hj)) + hj)
        pad = (16 - (12 + len(hj)) % 16) % 16
        f.write(b"\0" * pad)
        f.write(wr.blob)
    # reference: deterministic 3 s signal (tones, chirp, noise bursts)
    rng = np.random.default_rng(0)
    t = np.arange(3 * SR) / SR
    x = 0.3 * np.sin(2 * np.pi * (300 + 900 * t) * t) + 0.2 * np.sin(2 * np.pi * 1760 * t) * (t > 1.2)
    x += 0.05 * rng.standard_normal(len(t)) * ((t * 4) % 1 < 0.3)
    x = x.astype(np.float32)
    m = frontend_np(x.astype(np.float64), mel.astype(np.float64), win.astype(np.float64))
    # cross-check against EfficientAT's own front-end when it runs on this torch version
    try:
        from models.preprocess import AugmentMelSTFT
        pre = AugmentMelSTFT(n_mels=N_MELS, sr=SR, win_length=WIN, hopsize=HOP).eval()
        with torch.no_grad():
            mt = pre(torch.from_numpy(x)[None]).numpy()[0]
        print("front-end max |numpy - torch| =", float(np.max(np.abs(mt - m))))
    except Exception as e:  # noqa: BLE001
        print("EfficientAT front-end not runnable here:", e)
    with torch.no_grad():
        logits, emb = model(torch.from_numpy(m)[None, None])
    logits = logits.numpy().reshape(-1).astype(np.float32)
    emb = emb.numpy().reshape(-1).astype(np.float32)
    with open(f"{out}/{name}.ref.bin", "wb") as f:
        for a in (x, m.reshape(-1), emb, logits):
            f.write(struct.pack("<I", a.size))
            f.write(a.tobytes())
        f.write(struct.pack("<I", m.shape[1]))
    top = np.argsort(-logits)[:5]
    print(name, "params:", sum(p.numel() for p in model.parameters()), "embedding:", emb.size,
          "file:", os.path.getsize(f"{out}/{name}.rnm"), "bytes")
    print("reference top-5:", [(labels()[i], round(float(1 / (1 + np.exp(-logits[i]))), 3)) for i in top])


if __name__ == "__main__":
    main()
