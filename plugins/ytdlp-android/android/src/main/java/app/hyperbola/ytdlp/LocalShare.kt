package app.hyperbola.ytdlp

import android.app.Activity
import android.net.Uri
import android.provider.OpenableColumns
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import java.io.BufferedOutputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.NetworkInterface
import java.net.ServerSocket
import java.net.Socket
import java.security.SecureRandom

/**
 * Hands one finished file to another device on the same network.
 *
 * Bluetooth moves about 200 KB/s: fine for a song, forty minutes for a film.
 * A car head unit runs Android with a browser and is usually already on the
 * phone's hotspot, so the fastest thing the phone can offer is a plain link —
 * no pairing, no app on the other side, and full Wi-Fi speed.
 *
 * The server holds exactly one file, behind a random path, and stops itself.
 */
class LocalShare(private val activity: Activity) {
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private var server: ServerSocket? = null
    private var job: Job? = null

    @Volatile
    private var shared: Shared? = null

    private class Shared(val uri: Uri, val name: String, val size: Long, val token: String)

    /** Starts serving [uri] and returns the address to type on the other device. */
    @Synchronized
    fun start(uri: Uri): String {
        stop()
        val name = displayName(uri) ?: "download"
        val size = sizeOf(uri)
        val token = randomToken()
        shared = Shared(uri, name, size, token)

        val socket = ServerSocket(0)
        socket.reuseAddress = true
        server = socket
        job = scope.launch {
            while (!socket.isClosed) {
                val client = try {
                    socket.accept()
                } catch (e: Exception) {
                    break
                }
                launch { handle(client) }
            }
        }
        val address = localAddress() ?: throw IllegalStateException("this phone is not on a network")
        return "http://$address:${socket.localPort}/$token"
    }

    @Synchronized
    fun stop() {
        runCatching { server?.close() }
        server = null
        job?.cancel()
        job = null
        shared = null
    }

    @Synchronized
    fun isRunning(): Boolean = server?.isClosed == false

    private fun handle(client: Socket) {
        client.use { socket ->
            val request = socket.getInputStream().bufferedReader().readLine() ?: return
            val path = request.split(" ").getOrNull(1) ?: return
            val file = shared ?: return respond(socket.getOutputStream(), 503, "text/plain", "nothing is being shared".toByteArray())

            when {
                path == "/${file.token}" -> respond(
                    socket.getOutputStream(),
                    200,
                    "text/html; charset=utf-8",
                    page(file).toByteArray(),
                )
                path == "/${file.token}/file" -> stream(socket.getOutputStream(), file)
                else -> respond(socket.getOutputStream(), 404, "text/plain", "not here".toByteArray())
            }
        }
    }

    private fun page(file: Shared): String {
        val megabytes = if (file.size > 0) "%.1f MB".format(file.size / 1048576.0) else ""
        return """
            <!doctype html><html><head><meta charset="utf-8">
            <meta name="viewport" content="width=device-width,initial-scale=1">
            <title>${escape(file.name)}</title>
            <style>
              body{background:#0b1020;color:#e6ecf7;font-family:system-ui,sans-serif;
                   display:flex;min-height:100vh;align-items:center;justify-content:center;margin:0}
              .card{text-align:center;padding:32px}
              a{display:inline-block;margin-top:20px;padding:16px 28px;border-radius:12px;
                background:linear-gradient(120deg,#22d3ee,#3b82f6 55%,#a78bfa);
                color:#06121f;font-weight:700;font-size:20px;text-decoration:none}
              p{color:#93a1bd}
            </style></head><body><div class="card">
            <h2>${escape(file.name)}</h2><p>$megabytes</p>
            <a href="/${file.token}/file" download>Download</a>
            </div></body></html>
        """.trimIndent()
    }

    private fun stream(out: OutputStream, file: Shared) {
        val input = activity.contentResolver.openInputStream(file.uri)
            ?: return respond(out, 404, "text/plain", "the file is gone".toByteArray())
        val header = StringBuilder()
            .append("HTTP/1.1 200 OK\r\n")
            .append("Content-Type: application/octet-stream\r\n")
            .append("Content-Disposition: attachment; filename=\"${file.name.replace('"', '_')}\"\r\n")
        if (file.size > 0) header.append("Content-Length: ${file.size}\r\n")
        header.append("Connection: close\r\n\r\n")

        val sink = BufferedOutputStream(out, 64 * 1024)
        sink.write(header.toString().toByteArray())
        input.use { it.copyTo(sink, 64 * 1024) }
        sink.flush()
    }

    private fun respond(out: OutputStream, code: Int, type: String, body: ByteArray) {
        val header = "HTTP/1.1 $code\r\nContent-Type: $type\r\nContent-Length: ${body.size}\r\n" +
            "Connection: close\r\n\r\n"
        out.write(header.toByteArray())
        out.write(body)
        out.flush()
    }

    private fun displayName(uri: Uri): String? = runCatching {
        activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { if (it.moveToFirst()) it.getString(0) else null }
    }.getOrNull()

    private fun sizeOf(uri: Uri): Long = runCatching {
        activity.contentResolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)
            ?.use { if (it.moveToFirst()) it.getLong(0) else 0L } ?: 0L
    }.getOrDefault(0L)

    /** The address another device on this network can reach. */
    private fun localAddress(): String? {
        val candidates = mutableListOf<String>()
        for (nic in NetworkInterface.getNetworkInterfaces()) {
            if (!nic.isUp || nic.isLoopback) continue
            for (address in nic.inetAddresses) {
                if (address.isLoopbackAddress || address !is InetAddress) continue
                val text = address.hostAddress ?: continue
                if (text.contains(':')) continue
                // A hotspot interface is as good as Wi-Fi: the car is on it.
                if (nic.name.startsWith("wlan") || nic.name.startsWith("ap") || nic.name.startsWith("swlan")) {
                    return text
                }
                candidates.add(text)
            }
        }
        return candidates.firstOrNull()
    }

    private fun randomToken(): String {
        val bytes = ByteArray(9)
        SecureRandom().nextBytes(bytes)
        return bytes.joinToString("") { "%02x".format(it) }
    }

    private fun escape(text: String) = text
        .replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
}
