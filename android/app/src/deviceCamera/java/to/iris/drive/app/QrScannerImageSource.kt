package to.iris.drive.app

import android.content.Context
import android.media.Image
import com.google.mlkit.vision.common.InputImage

@Suppress("UNUSED_PARAMETER")
internal fun qrScannerImageSource(context: Context): (Image, Int) -> InputImage? =
    InputImage::fromMediaImage
