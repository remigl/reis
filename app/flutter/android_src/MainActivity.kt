package org.reis.reis

// Native helpers for building a Reis library on the phone itself:
//  * listAudio   : every audio file known to Android (MediaStore), with tags and folder
//  * decode      : any format Android can play (MP3, AAC, FLAC, OGG, Opus, WAV…) -> mono 16-bit PCM
//  * thumbnail   : album art / embedded cover as a small JPEG
//  * readSidecar : best-effort read of a .lrc next to the audio file
// Copied over the generated MainActivity by scripts/setup_flutter.sh.

import android.Manifest
import android.content.ContentUris
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Size
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors

class MainActivity : FlutterActivity() {
    private val io = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private var pendingPermission: MethodChannel.Result? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "reis/audio").setMethodCallHandler { call, result ->
            when (call.method) {
                "requestPermission" -> requestPermission(result)
                "listAudio" -> background(result) { listAudio() }
                "decode" -> background(result) {
                    decode(call.argument<String>("source")!!, call.argument<Double>("maxSeconds") ?: 900.0)
                }
                "thumbnail" -> background(result) {
                    thumbnail(call.argument<String>("source")!!, call.argument<Int>("size") ?: 160)
                }
                "readSidecar" -> background(result) {
                    readSidecar(call.argument<String>("path") ?: "", call.argument<String>("ext") ?: "lrc")
                }
                else -> result.notImplemented()
            }
        }
    }

    private fun background(result: MethodChannel.Result, work: () -> Any?) {
        io.execute {
            try {
                val v = work()
                main.post { result.success(v) }
            } catch (e: Throwable) {
                main.post { result.error("reis", e.message ?: e.toString(), null) }
            }
        }
    }

    // ---------------------------------------------------------------- permission

    private fun audioPermission(): String =
        if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_AUDIO else Manifest.permission.READ_EXTERNAL_STORAGE

    private fun requestPermission(result: MethodChannel.Result) {
        if (checkSelfPermission(audioPermission()) == PackageManager.PERMISSION_GRANTED) {
            result.success(true)
            return
        }
        pendingPermission = result
        requestPermissions(arrayOf(audioPermission()), 4242)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 4242) {
            pendingPermission?.success(grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED)
            pendingPermission = null
        }
    }

    // ---------------------------------------------------------------- MediaStore

    @Suppress("DEPRECATION")
    private fun listAudio(): List<Map<String, Any?>> {
        val out = ArrayList<Map<String, Any?>>()
        val uri = MediaStore.Audio.Media.EXTERNAL_CONTENT_URI
        val cols = mutableListOf(
            MediaStore.Audio.Media._ID, MediaStore.Audio.Media.TITLE, MediaStore.Audio.Media.ARTIST,
            MediaStore.Audio.Media.ALBUM, MediaStore.Audio.Media.DURATION, MediaStore.Audio.Media.DISPLAY_NAME,
            MediaStore.Audio.Media.IS_MUSIC, MediaStore.Audio.Media.DATA,
        )
        if (Build.VERSION.SDK_INT >= 29) cols.add(MediaStore.Audio.Media.RELATIVE_PATH)
        contentResolver.query(uri, cols.toTypedArray(), null, null, MediaStore.Audio.Media.DISPLAY_NAME)?.use { c ->
            val iId = c.getColumnIndexOrThrow(MediaStore.Audio.Media._ID)
            val iTitle = c.getColumnIndexOrThrow(MediaStore.Audio.Media.TITLE)
            val iArtist = c.getColumnIndexOrThrow(MediaStore.Audio.Media.ARTIST)
            val iAlbum = c.getColumnIndexOrThrow(MediaStore.Audio.Media.ALBUM)
            val iDur = c.getColumnIndexOrThrow(MediaStore.Audio.Media.DURATION)
            val iName = c.getColumnIndexOrThrow(MediaStore.Audio.Media.DISPLAY_NAME)
            val iMusic = c.getColumnIndexOrThrow(MediaStore.Audio.Media.IS_MUSIC)
            val iData = c.getColumnIndex(MediaStore.Audio.Media.DATA)
            val iRel = if (Build.VERSION.SDK_INT >= 29) c.getColumnIndex(MediaStore.Audio.Media.RELATIVE_PATH) else -1
            while (c.moveToNext()) {
                val id = c.getLong(iId)
                val data = if (iData >= 0) c.getString(iData) else null
                val folder = when {
                    iRel >= 0 && c.getString(iRel) != null -> c.getString(iRel).trimEnd('/')
                    data != null -> File(data).parent ?: ""
                    else -> ""
                }
                val artist = c.getString(iArtist)
                out.add(mapOf(
                    "source" to ContentUris.withAppendedId(uri, id).toString(),
                    "path" to data,
                    "title" to c.getString(iTitle),
                    "artist" to if (artist == null || artist == "<unknown>") null else artist,
                    "album" to c.getString(iAlbum),
                    "durationMs" to c.getLong(iDur),
                    "fileName" to c.getString(iName),
                    "folder" to folder,
                    "isMusic" to (c.getInt(iMusic) != 0),
                ))
            }
        }
        return out
    }

    // ---------------------------------------------------------------- decoding

    private fun decode(source: String, maxSeconds: Double): Map<String, Any> {
        val ex = MediaExtractor()
        try {
            if (source.startsWith("content://")) ex.setDataSource(this, Uri.parse(source), null) else ex.setDataSource(source)
            var track = -1
            var fmt: MediaFormat? = null
            for (i in 0 until ex.trackCount) {
                val f = ex.getTrackFormat(i)
                if (f.getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true) {
                    track = i; fmt = f; break
                }
            }
            if (track < 0 || fmt == null) throw IllegalArgumentException("no audio track")
            ex.selectTrack(track)
            val codec = MediaCodec.createDecoderByType(fmt.getString(MediaFormat.KEY_MIME)!!)
            codec.configure(fmt, null, null, 0)
            codec.start()
            var rate = fmt.getInteger(MediaFormat.KEY_SAMPLE_RATE)
            var channels = fmt.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
            var encoding = AudioFormat.ENCODING_PCM_16BIT
            val out = ByteArrayOutputStream(1 shl 20)
            val info = MediaCodec.BufferInfo()
            var inputDone = false
            var done = false
            var frames = 0L
            val maxFrames = (maxSeconds * rate).toLong()
            try {
                while (!done) {
                    if (!inputDone) {
                        val ii = codec.dequeueInputBuffer(10_000)
                        if (ii >= 0) {
                            val buf = codec.getInputBuffer(ii)!!
                            val n = ex.readSampleData(buf, 0)
                            if (n < 0) {
                                codec.queueInputBuffer(ii, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                                inputDone = true
                            } else {
                                codec.queueInputBuffer(ii, 0, n, ex.sampleTime, 0)
                                ex.advance()
                            }
                        }
                    }
                    val oi = codec.dequeueOutputBuffer(info, 10_000)
                    if (oi == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                        val f = codec.outputFormat
                        rate = f.getInteger(MediaFormat.KEY_SAMPLE_RATE)
                        channels = f.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
                        if (Build.VERSION.SDK_INT >= 24 && f.containsKey(MediaFormat.KEY_PCM_ENCODING)) {
                            encoding = f.getInteger(MediaFormat.KEY_PCM_ENCODING)
                        }
                    } else if (oi >= 0) {
                        val buf = codec.getOutputBuffer(oi)!!
                        buf.position(info.offset)
                        buf.limit(info.offset + info.size)
                        frames += downmix(buf.slice().order(ByteOrder.LITTLE_ENDIAN), channels, encoding, out)
                        codec.releaseOutputBuffer(oi, false)
                        if ((info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0 || frames >= maxFrames) done = true
                    }
                }
            } finally {
                codec.stop()
                codec.release()
            }
            return mapOf("pcm" to out.toByteArray(), "rate" to rate)
        } finally {
            ex.release()
        }
    }

    /** Average channels into mono 16-bit little-endian. Returns frames written. */
    private fun downmix(buf: ByteBuffer, channels: Int, encoding: Int, out: ByteArrayOutputStream): Long {
        val ch = channels.coerceAtLeast(1)
        val tmp: ByteBuffer
        val frames: Int
        if (encoding == AudioFormat.ENCODING_PCM_FLOAT) {
            val fb = buf.asFloatBuffer()
            frames = fb.remaining() / ch
            tmp = ByteBuffer.allocate(frames * 2).order(ByteOrder.LITTLE_ENDIAN)
            for (i in 0 until frames) {
                var s = 0f
                for (k in 0 until ch) s += fb.get(i * ch + k)
                tmp.putShort(((s / ch).coerceIn(-1f, 1f) * 32767f).toInt().toShort())
            }
        } else {
            val sb = buf.asShortBuffer()
            frames = sb.remaining() / ch
            tmp = ByteBuffer.allocate(frames * 2).order(ByteOrder.LITTLE_ENDIAN)
            for (i in 0 until frames) {
                var s = 0
                for (k in 0 until ch) s += sb.get(i * ch + k)
                tmp.putShort((s / ch).toShort())
            }
        }
        out.write(tmp.array(), 0, frames * 2)
        return frames.toLong()
    }

    // ---------------------------------------------------------------- artwork, sidecars

    private fun thumbnail(source: String, size: Int): ByteArray? {
        var bmp: Bitmap? = null
        if (source.startsWith("content://") && Build.VERSION.SDK_INT >= 29) {
            bmp = try { contentResolver.loadThumbnail(Uri.parse(source), Size(size, size), null) } catch (_: Throwable) { null }
        }
        if (bmp == null) {
            val r = MediaMetadataRetriever()
            try {
                if (source.startsWith("content://")) r.setDataSource(this, Uri.parse(source)) else r.setDataSource(source)
                val pic = r.embeddedPicture
                if (pic != null) {
                    val full = BitmapFactory.decodeByteArray(pic, 0, pic.size)
                    if (full != null) bmp = Bitmap.createScaledBitmap(full, size, size, true)
                }
            } catch (_: Throwable) {
            } finally {
                r.release()
            }
        }
        val b = bmp ?: return null
        val o = ByteArrayOutputStream()
        b.compress(Bitmap.CompressFormat.JPEG, 80, o)
        return o.toByteArray()
    }

    private fun readSidecar(path: String, ext: String): String? {
        if (path.isEmpty()) return null
        val dot = path.lastIndexOf('.')
        val f = File((if (dot > 0) path.substring(0, dot) else path) + "." + ext)
        return try { if (f.canRead()) f.readText() else null } catch (_: Throwable) { null }
    }
}
