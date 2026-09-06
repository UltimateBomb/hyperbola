package app.hyperbola.ytdlp

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.system.Os
import android.content.ContentValues
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.documentfile.provider.DocumentFile
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.annotation.ActivityCallback
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.yausername.ffmpeg.FFmpeg
import com.yausername.youtubedl_android.YoutubeDL
import com.yausername.youtubedl_android.YoutubeDLRequest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue

@InvokeArg
class ProbeArgs {
    lateinit var url: String
    var args: Array<String> = arrayOf()
}

@InvokeArg
class DownloadArgs {
    lateinit var id: String
    lateinit var url: String
    var args: Array<String> = arrayOf()
    lateinit var outputDir: String
}

@InvokeArg
class ProcessArgs {
    lateinit var id: String
}

@InvokeArg
class UpdateArgs {
    var channel: String = "stable"
}

@InvokeArg
class PublishArgs {
    lateinit var sourcePath: String
    var treeUri: String? = null
}

/**
 * Bridges Rust to youtubedl-android.
 *
 * The engine on the Rust side builds the same argument list it builds for the
 * desktop and reads the same output lines back, so one parser and one queue
 * serve both platforms. Everything Android-specific — the bundled Python, the
 * Storage Access Framework, the update channel — is handled here.
 */
@TauriPlugin
class YtdlpPlugin(private val activity: Activity) : Plugin(activity) {
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    /** Output lines per process, drained by the Rust side as it polls. */
    private val output = ConcurrentHashMap<String, ConcurrentLinkedQueue<String>>()
    private val finished = ConcurrentHashMap<String, Boolean>()

    @Volatile
    private var ready = false

    @Volatile
    private var initError: String? = null

    override fun load(webView: WebView) {
        scope.launch {
            try {
                YoutubeDL.getInstance().init(activity.application)
                FFmpeg.getInstance().init(activity.application)
                ready = true
            } catch (e: Exception) {
                initError = e.message ?: "failed to initialise the download engine"
            }
        }
    }

    private fun requireEngine(invoke: Invoke): Boolean {
        if (ready) return true
        invoke.reject(initError ?: "the download engine is still starting")
        return false
    }

    private fun request(url: String, args: Array<String>): YoutubeDLRequest {
        val request = YoutubeDLRequest(url)
        // The engine emits a flat argument vector; youtubedl-android takes
        // options one token at a time.
        args.forEach { request.addOption(it) }
        return request
    }

    @Command
    fun probe(invoke: Invoke) {
        if (!requireEngine(invoke)) return
        val args = invoke.parseArgs(ProbeArgs::class.java)
        scope.launch {
            try {
                val response = YoutubeDL.getInstance().execute(request(args.url, args.args))
                val result = JSObject()
                result.put("json", response.out)
                invoke.resolve(result)
            } catch (e: Exception) {
                invoke.reject(e.message ?: "could not read media info")
            }
        }
    }

    @Command
    fun download(invoke: Invoke) {
        if (!requireEngine(invoke)) return
        val args = invoke.parseArgs(DownloadArgs::class.java)
        val lines = ConcurrentLinkedQueue<String>()
        output[args.id] = lines
        finished[args.id] = false
        scope.launch {
            try {
                File(args.outputDir).mkdirs()
                val response = YoutubeDL.getInstance()
                    .execute(request(args.url, args.args), args.id, true) { _, _, line ->
                        lines.add(line)
                    }
                val result = JSObject()
                result.put("exitCode", response.exitCode)
                // Errors are folded into the output stream, so a failure with
                // an empty `err` would reach the user as a blank message.
                val details = response.err.ifBlank { response.out }
                result.put("stderr", details.takeLast(4000))
                invoke.resolve(result)
            } catch (e: Exception) {
                invoke.reject(e.message ?: "download failed")
            } finally {
                finished[args.id] = true
            }
        }
    }

    /** Hands over everything printed since the previous call. */
    @Command
    fun pollOutput(invoke: Invoke) {
        val args = invoke.parseArgs(ProcessArgs::class.java)
        val queue = output[args.id]
        val collected = JSArray()
        if (queue != null) {
            while (true) {
                val line = queue.poll() ?: break
                collected.put(line)
            }
        }
        val done = finished[args.id] ?: true
        if (done && (queue?.isEmpty() != false)) {
            output.remove(args.id)
            finished.remove(args.id)
        }
        val result = JSObject()
        result.put("lines", collected)
        result.put("finished", done)
        invoke.resolve(result)
    }

    @Command
    fun cancel(invoke: Invoke) {
        val args = invoke.parseArgs(ProcessArgs::class.java)
        YoutubeDL.getInstance().destroyProcessById(args.id)
        finished[args.id] = true
        invoke.resolve(JSObject())
    }

    /**
     * Makes ffmpeg reachable by the name yt-dlp looks for.
     *
     * Android only executes code from the native library directory, where
     * ffmpeg is installed as `libffmpeg.so`. yt-dlp searches for a file
     * called `ffmpeg`, finds nothing, and fails at the merge step after the
     * whole file has downloaded. Symlinks in the app's own directory give it
     * the name it expects while the executable stays where Android allows it.
     */
    @Command
    fun enginePaths(invoke: Invoke) {
        val result = JSObject()
        try {
            val binDir = File(activity.filesDir, "engine-bin").apply { mkdirs() }
            val nativeDir = File(activity.applicationInfo.nativeLibraryDir)
            link(File(nativeDir, "libffmpeg.so"), File(binDir, "ffmpeg"))
            link(File(nativeDir, "libffprobe.so"), File(binDir, "ffprobe"))
            val ffmpeg = File(binDir, "ffmpeg")
            result.put("ffmpegDir", if (ffmpeg.exists()) binDir.absolutePath else null)
        } catch (e: Exception) {
            result.put("ffmpegDir", null)
        }
        invoke.resolve(result)
    }

    private fun link(target: File, linkFile: File) {
        if (!target.exists()) return
        linkFile.delete()
        Os.symlink(target.absolutePath, linkFile.absolutePath)
    }

    @Command
    fun engineVersion(invoke: Invoke) {
        val result = JSObject()
        result.put("version", YoutubeDL.getInstance().version(activity))
        invoke.resolve(result)
    }

    /** The Android counterpart of downloading a fresh yt-dlp binary. */
    @Command
    fun updateEngine(invoke: Invoke) {
        if (!requireEngine(invoke)) return
        val args = invoke.parseArgs(UpdateArgs::class.java)
        val channel = when (args.channel.lowercase()) {
            "nightly" -> YoutubeDL.UpdateChannel.NIGHTLY
            else -> YoutubeDL.UpdateChannel.STABLE
        }
        scope.launch {
            try {
                val status = YoutubeDL.getInstance().updateYoutubeDL(activity.application, channel)
                val result = JSObject()
                result.put("status", status?.name ?: "UNKNOWN")
                result.put("version", YoutubeDL.getInstance().version(activity))
                invoke.resolve(result)
            } catch (e: Exception) {
                invoke.reject(e.message ?: "could not update the engine")
            }
        }
    }

    @Command
    fun pickOutputFolder(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
        }
        startActivityForResult(invoke, intent, "outputFolderPicked")
    }

    @ActivityCallback
    fun outputFolderPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        val response = JSObject()
        if (uri == null) {
            response.put("uri", null)
            response.put("label", null)
            invoke.resolve(response)
            return
        }
        // Without this the folder is forgotten the moment the app restarts.
        activity.contentResolver.takePersistableUriPermission(
            uri,
            Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
        )
        response.put("uri", uri.toString())
        response.put("label", DocumentFile.fromTreeUri(activity, uri)?.name)
        invoke.resolve(response)
    }

    /**
     * Moves a finished file out of the app's private storage.
     *
     * yt-dlp writes to a plain path, which on Android may only be inside the
     * app's own directory. A file left there disappears with the app and is
     * invisible to every other app, so it is copied into the folder the user
     * chose — or into Downloads when they chose none.
     */
    @Command
    fun publish(invoke: Invoke) {
        val args = invoke.parseArgs(PublishArgs::class.java)
        scope.launch {
            try {
                val source = File(args.sourcePath)
                if (!source.isFile) {
                    invoke.reject("finished file not found: ${args.sourcePath}")
                    return@launch
                }
                val display = args.treeUri?.let { copyIntoTree(source, Uri.parse(it)) }
                    ?: copyIntoDownloads(source)
                source.delete()
                val result = JSObject()
                result.put("displayPath", display)
                invoke.resolve(result)
            } catch (e: Exception) {
                invoke.reject(e.message ?: "could not save the file")
            }
        }
    }

    private fun copyIntoTree(source: File, treeUri: Uri): String {
        val tree = DocumentFile.fromTreeUri(activity, treeUri)
            ?: throw IllegalStateException("the chosen folder is no longer available")
        tree.findFile(source.name)?.delete()
        val target = tree.createFile(mimeTypeOf(source.name), source.name)
            ?: throw IllegalStateException("could not create the file in the chosen folder")
        activity.contentResolver.openOutputStream(target.uri).use { out ->
            requireNotNull(out) { "could not open the target file" }
            source.inputStream().use { it.copyTo(out) }
        }
        return "${tree.name ?: "folder"}/${source.name}"
    }

    private fun copyIntoDownloads(source: File): String {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val values = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, source.name)
                put(MediaStore.Downloads.MIME_TYPE, mimeTypeOf(source.name))
                put(MediaStore.Downloads.IS_PENDING, 1)
            }
            val resolver = activity.contentResolver
            val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: throw IllegalStateException("could not add the file to Downloads")
            resolver.openOutputStream(uri).use { out ->
                requireNotNull(out) { "could not open the target file" }
                source.inputStream().use { it.copyTo(out) }
            }
            values.clear()
            values.put(MediaStore.Downloads.IS_PENDING, 0)
            resolver.update(uri, values, null, null)
            return "Downloads/${source.name}"
        }
        val downloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
        downloads.mkdirs()
        val target = File(downloads, source.name)
        source.inputStream().use { input -> target.outputStream().use { input.copyTo(it) } }
        return target.absolutePath
    }

    private fun mimeTypeOf(name: String): String = when (name.substringAfterLast('.', "").lowercase()) {
        "mp4", "m4v" -> "video/mp4"
        "webm" -> "video/webm"
        "mkv" -> "video/x-matroska"
        "mp3" -> "audio/mpeg"
        "m4a" -> "audio/mp4"
        "opus", "ogg" -> "audio/ogg"
        "flac" -> "audio/flac"
        "wav" -> "audio/wav"
        else -> "application/octet-stream"
    }
}
