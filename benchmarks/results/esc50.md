wrote mn10.reis: 15.85 MB, 107 min of audio in 139.5 s (46x real time)
  semantic: model mn10_as, 3603 vectors x 960 dims, window 3 s; threshold 0.605 (≤ 20% unknown sounds accepted), leave-one-out accuracy 93.2%

### mn10 (threshold 0.605)
| condition | closed-set top-1 | top-5 | correct @ calibrated threshold | wrong label | unknown accepted | correct @ 5 % unknown accepted |
|---|---|---|---|---|---|---|
| clean | 88.8 % | 97.8 % | 57.2 % | 3.1 % | 7.5 % | 55.0 % |
| speaker | 75.9 % | 96.2 % | 29.1 % | 0.9 % | 0.0 % | 32.8 % |
| noise10 | 73.8 % | 94.1 % | 39.7 % | 5.0 % | 3.8 % | 40.3 % |
| speaker_noise5 | 55.0 % | 84.1 % | 16.9 % | 7.5 % | 5.0 % | 17.8 % |
time per query p50 104.8 ms, p95 133.9 ms
wrote mn10_augment.reis: 17.27 MB, 107 min of audio in 327.6 s (20x real time)
  semantic: model mn10_as, 7290 vectors x 960 dims, window 3 s; threshold 0.675 (≤ 20% unknown sounds accepted), leave-one-out accuracy 90.2%

### mn10_augment (threshold 0.675)
| condition | closed-set top-1 | top-5 | correct @ calibrated threshold | wrong label | unknown accepted | correct @ 5 % unknown accepted |
|---|---|---|---|---|---|---|
| clean | 89.4 % | 96.9 % | 42.2 % | 3.1 % | 3.8 % | 50.0 % |
| speaker | 81.9 % | 97.8 % | 21.2 % | 3.4 % | 1.2 % | 35.0 % |
| noise10 | 76.2 % | 95.3 % | 24.1 % | 0.0 % | 1.2 % | 42.2 % |
| speaker_noise5 | 58.4 % | 89.4 % | 9.4 % | 0.0 % | 0.0 % | 17.5 % |
time per query p50 116.8 ms, p95 147.8 ms
wrote mn04.reis: 3.81 MB, 107 min of audio in 45.0 s (142x real time)
  semantic: model mn04_as, 3603 vectors x 384 dims, window 3 s; threshold 0.686 (≤ 20% unknown sounds accepted), leave-one-out accuracy 91.6%

### mn04 (threshold 0.686)
| condition | closed-set top-1 | top-5 | correct @ calibrated threshold | wrong label | unknown accepted | correct @ 5 % unknown accepted |
|---|---|---|---|---|---|---|
| clean | 85.9 % | 98.8 % | 69.1 % | 3.4 % | 26.2 % | 38.1 % |
| speaker | 68.8 % | 95.9 % | 35.3 % | 4.7 % | 12.5 % | 16.6 % |
| noise10 | 66.2 % | 94.1 % | 45.0 % | 15.6 % | 21.2 % | 28.1 % |
| speaker_noise5 | 46.6 % | 84.1 % | 16.2 % | 20.0 % | 21.2 % | 8.4 % |
time per query p50 56.0 ms, p95 69.1 ms

### exact engine, same recording (3 s excerpts of references)
clean: top-1 (accepted and correct): 89.6%
speaker: top-1 (accepted and correct): 48.1%
noise10: top-1 (accepted and correct): 70.5%
speaker_noise5: top-1 (accepted and correct): 25.7%
other recordings of the same category (clean): top-1 (accepted and correct): 0.6%
