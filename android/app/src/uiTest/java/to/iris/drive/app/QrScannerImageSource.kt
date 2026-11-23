package to.iris.drive.app

import android.content.Context
import android.graphics.BitmapFactory
import android.media.Image
import com.google.mlkit.vision.common.InputImage
import java.io.File
import java.security.MessageDigest

internal fun qrScannerImageSource(context: Context): (Image, Int) -> InputImage? {
    val expectedFile = File(context.filesDir, "qr-frame.expected")
    if (!expectedFile.exists()) return InputImage::fromMediaImage
    val expected = expectedFile.readText().trim()
    require(expected.matches(Regex("[a-f0-9]{64}")))
    val frame = File(context.filesDir, "qr-frame-$expected.png")
    var input: InputImage? = null
    return { _, _ ->
        if (input == null && frame.length() in 1..4_194_304L) {
            val bytes = frame.readBytes()
            val digest = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
            if (digest == expected) {
                val size = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size, size)
                require(size.outWidth in 1..4096 && size.outHeight in 1..4096)
                val bitmap = requireNotNull(BitmapFactory.decodeByteArray(bytes, 0, bytes.size))
                input = InputImage.fromBitmap(bitmap, 0)
                frame.delete()
                expectedFile.delete()
            }
        }
        input
    }
}
