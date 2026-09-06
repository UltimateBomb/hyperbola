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
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import android.content.pm.PackageManager
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
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
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
class ServiceArgs {
    var text: String = "Downloading"
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

    /**
     * The last lines of each run, kept even though polling drains the queue.
     * Without them a failure has nothing to say for itself: the engine
     * library throws with an empty message when yt-dlp exits non-zero.
     */
    private val tails = ConcurrentHashMap<String, ArrayDeque<String>>()

    /** Completed once the engine has unpacked itself, successfully or not. */
    private val started = CompletableDeferred<Unit>()

    @Volatile
    private var initError: String? = null

    override fun load(webView: WebView) {
        scope.launch {
            try {
                YoutubeDL.getInstance().init(activity.application)
                FFmpeg.getInstance().init(activity.application)
            } catch (e: Exception) {
                initError = e.message ?: "failed to initialise the download engine"
            } finally {
                started.complete(Unit)
            }
        }
    }

    /**
     * Waits for the engine instead of refusing.
     *
     * Unpacking Python and ffmpeg takes several seconds on first launch, and
     * the app checks for updates as soon as it opens — refusing during that
     * window turned a slow start into a failure the user had to retry.
     */
    private suspend fun awaitEngine(invoke: Invoke): Boolean {
        if (withTimeoutOrNull(90_000) { started.await() } == null) {
            invoke.reject("the download engine did not finish starting")
            return false
        }
        initError?.let {
            invoke.reject(it)
            return false
        }
        return true
    }

    private fun request(url: String, args: Array<String>): YoutubeDLRequest {
        val request = YoutubeDLRequest(url)
        // addCommands appends the arguments verbatim. addOption must not be
        // used here: it keys options by name in a map, which folds repeated
        // flags together and separates values from the flags they belong to —
        // yt-dlp then sees the stray values as URLs and refuses them.
        request.addCommands(args.toList())
        return request
    }

    @Command
    fun probe(invoke: Invoke) {
        val args = invoke.parseArgs(ProbeArgs::class.java)
        scope.launch {
            if (!awaitEngine(invoke)) return@launch
            try {
                val response = YoutubeDL.getInstance().execute(request(args.url, args.args))
                val result = JSObject()
                result.put("json", response.out)
                invoke.resolve(result)
            } catch (e: Exception) {
                invoke.reject(describe(e, "could not read media info"))
            }
        }
    }

    @Command
    fun download(invoke: Invoke) {
        val args = invoke.parseArgs(DownloadArgs::class.java)
        val lines = ConcurrentLinkedQueue<String>()
        val tail = ArrayDeque<String>()
        output[args.id] = lines
        tails[args.id] = tail
        finished[args.id] = false
        scope.launch {
            if (!awaitEngine(invoke)) {
                finished[args.id] = true
                return@launch
            }
            try {
                File(args.outputDir).mkdirs()
                val response = YoutubeDL.getInstance()
                    .execute(request(args.url, args.args), args.id, true) { _, _, line ->
                        lines.add(line)
                        synchronized(tail) {
                            tail.addLast(line)
                            while (tail.size > 80) tail.removeFirst()
                        }
                    }
                val result = JSObject()
                result.put("exitCode", response.exitCode)
                // Errors are folded into the output stream, so a failure with
                // an empty `err` would reach the user as a blank message.
                val details = response.err.ifBlank { response.out }
                result.put("stderr", details.takeLast(4000))
                invoke.resolve(result)
            } catch (e: Exception) {
                // The library throws whenever yt-dlp exits non-zero, which it
                // does after an ignored postprocessing error — with the
                // finished file already on disk. Report the exit instead of
                // failing the call, and let the caller judge by the file.
                if (e.javaClass.simpleName == "CanceledException") {
                    invoke.reject("cancelled")
                } else {
                    val result = JSObject()
                    result.put("exitCode", 1)
                    val collected = synchronized(tail) { tail.joinToString("\n") }
                    result.put("stderr", describe(e, "the download failed") + "\n" + collected)
                    invoke.resolve(result)
                }
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
            tails.remove(args.id)
        }
        val result = JSObject()
        result.put("lines", collected)
        result.put("finished", done)
        invoke.resolve(result)
    }

    /**
     * Starts the foreground service that keeps downloads alive while the
     * screen is off, and asks for the notification permission the first time
     * — without it the work still runs but the user cannot see or stop it.
     */
    @Command
    fun startDownloadService(invoke: Invoke) {
        val args = invoke.parseArgs(ServiceArgs::class.java)
        if (Build.VERSION.SDK_INT >= 33) {
            val granted = ContextCompat.checkSelfPermission(
                activity,
                "android.permission.POST_NOTIFICATIONS",
            ) == PackageManager.PERMISSION_GRANTED
            if (!granted) {
                ActivityCompat.requestPermissions(
                    activity,
                    arrayOf("android.permission.POST_NOTIFICATIONS"),
                    REQUEST_NOTIFICATIONS,
                )
            }
        }
        DownloadService.start(activity, args.text)
        invoke.resolve(JSObject())
    }

    @Command
    fun stopDownloadService(invoke: Invoke) {
        DownloadService.stop(activity)
        invoke.resolve(JSObject())
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

    /**
     * A reason, always.
     *
     * The engine library throws exceptions whose message is empty, and an
     * empty rejection reached the user as a failed download with nothing
     * written next to it.
     */
    private companion object {
        const val REQUEST_NOTIFICATIONS = 4711
    }

    private fun describe(e: Exception, fallback: String): String {
        val own = e.message?.trim().orEmpty()
        if (own.isNotEmpty()) return own
        val cause = e.cause?.message?.trim().orEmpty()
        if (cause.isNotEmpty()) return "${e.javaClass.simpleName}: $cause"
        return "$fallback (${e.javaClass.simpleName})"
    }

    private fun link(target: File, linkFile: File) {
        if (!target.exists()) return
        linkFile.delete()
        Os.symlink(target.absolutePath, linkFile.absolutePath)
    }

    /**
     * The engine's own yt-dlp version. Waits for the unpack: answering "no
     * version" during startup makes the app announce that nothing can
     * download, on a phone where the engine is built in.
     */
    @Command
    fun engineVersion(invoke: Invoke) {
        scope.launch {
            if (!awaitEngine(invoke)) return@launch
            val result = JSObject()
            result.put("version", YoutubeDL.getInstance().version(activity))
            invoke.resolve(result)
        }
    }

    /** The Android counterpart of downloading a fresh yt-dlp binary. */
    @Command
    fun updateEngine(invoke: Invoke) {
        val args = invoke.parseArgs(UpdateArgs::class.java)
        val channel = when (args.channel.lowercase()) {
            "nightly" -> YoutubeDL.UpdateChannel.NIGHTLY
            else -> YoutubeDL.UpdateChannel.STABLE
        }
        scope.launch {
            if (!awaitEngine(invoke)) return@launch
            try {
                val status = YoutubeDL.getInstance().updateYoutubeDL(activity.application, channel)
                val result = JSObject()
                result.put("status", status?.name ?: "UNKNOWN")
                result.put("version", YoutubeDL.getInstance().version(activity))
                invoke.resolve(result)
            } catch (e: Exception) {
                invoke.reject(describe(e, "could not update the engine"))
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
                invoke.reject(describe(e, "could not save the file"))
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
