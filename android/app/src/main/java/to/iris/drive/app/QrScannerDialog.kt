package to.iris.drive.app

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.OptIn
import androidx.camera.core.CameraSelector
import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

@OptIn(ExperimentalGetImage::class)
@Composable
internal fun QrScannerDialog(
    onDismiss: () -> Unit,
    onScanned: (String) -> String?,
) {
    QrScannerHost(
        onDismiss = onDismiss,
        onScanned = onScanned,
    ) { previewView, error ->
        Dialog(
            onDismissRequest = onDismiss,
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            QrScannerCamera(
                previewView = previewView,
                error = error,
                onDismiss = onDismiss,
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}

@OptIn(ExperimentalGetImage::class)
@Composable
private fun QrScannerHost(
    onDismiss: () -> Unit,
    onScanned: (String) -> String?,
    content: @Composable (PreviewView, String?) -> Unit,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    var error by remember { mutableStateOf<String?>(null) }
    var hasPermission by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }

    val permissionLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            hasPermission = granted
            if (!granted) {
                error = "Camera permission is needed to scan links."
            }
        }

    LaunchedEffect(Unit) {
        if (!hasPermission) {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    if (!hasPermission) {
        AlertDialog(
            onDismissRequest = onDismiss,
            title = { Text("Scan") },
            text = { Text(error ?: "Waiting for camera permission.") },
            confirmButton = {
                TextButton(onClick = { permissionLauncher.launch(Manifest.permission.CAMERA) }) {
                    Text("Retry")
                }
            },
            dismissButton = {
                TextButton(onClick = onDismiss) {
                    Text("Close")
                }
            },
        )
        return
    }

    val previewView =
        remember {
            PreviewView(context).apply {
                scaleType = PreviewView.ScaleType.FILL_CENTER
            }
        }
    val scanner =
        remember {
            val options =
                BarcodeScannerOptions.Builder()
                    .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                    .build()
            BarcodeScanning.getClient(options)
        }
    val analysisExecutor = remember { Executors.newSingleThreadExecutor() }
    val didEmit = remember { AtomicBoolean(false) }
    val inFlight = remember { AtomicBoolean(false) }
    var cameraProvider: ProcessCameraProvider? by remember { mutableStateOf(null) }

    DisposableEffect(Unit) {
        onDispose {
            runCatching { cameraProvider?.unbindAll() }
            runCatching { scanner.close() }
            runCatching { analysisExecutor.shutdown() }
        }
    }

    LaunchedEffect(Unit) {
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener(
            {
                runCatching { future.get() }
                    .onSuccess { cameraProvider = it }
                    .onFailure { error = "Camera scanner unavailable." }
            },
            ContextCompat.getMainExecutor(context),
        )
    }

    LaunchedEffect(cameraProvider) {
        val provider = cameraProvider ?: return@LaunchedEffect
        error = null
        didEmit.set(false)
        inFlight.set(false)

        val preview = Preview.Builder().build()
        preview.setSurfaceProvider(previewView.surfaceProvider)

        val analysis =
            ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()

        analysis.setAnalyzer(analysisExecutor) { imageProxy ->
            val mediaImage = imageProxy.image
            if (mediaImage == null) {
                imageProxy.close()
                return@setAnalyzer
            }
            if (!inFlight.compareAndSet(false, true)) {
                imageProxy.close()
                return@setAnalyzer
            }

            val image = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)
            scanner.process(image)
                .addOnSuccessListener { barcodes ->
                    val raw = barcodes.firstOrNull()?.rawValue?.trim().orEmpty()
                    if (raw.isBlank()) {
                        return@addOnSuccessListener
                    }
                    if (!didEmit.compareAndSet(false, true)) {
                        return@addOnSuccessListener
                    }

                    val errorMessage = onScanned(raw)
                    if (errorMessage != null) {
                        didEmit.set(false)
                        error = errorMessage
                    }
                }.addOnFailureListener {
                    // Keep scanning while the camera remains open.
                }.addOnCompleteListener {
                    inFlight.set(false)
                    imageProxy.close()
                }
        }

        runCatching {
            provider.unbindAll()
            provider.bindToLifecycle(
                lifecycleOwner,
                CameraSelector.DEFAULT_BACK_CAMERA,
                preview,
                analysis,
            )
        }.onFailure {
            error = "Camera scanner unavailable."
        }
    }

    content(previewView, error)
}

@Composable
private fun QrScannerCamera(
    previewView: PreviewView,
    error: String?,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier.background(Color.Black)) {
        AndroidView(
            factory = { previewView },
            modifier = Modifier.fillMaxSize(),
        )
        QrScannerCrosshair(modifier = Modifier.fillMaxSize())
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .padding(horizontal = 4.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TextButton(onClick = onDismiss) {
                Text("Close", color = Color.White)
            }
            Text(
                text = "Scan",
                style = MaterialTheme.typography.titleLarge,
                color = Color.White,
            )
        }
        error?.let { message ->
            Text(
                text = message,
                modifier =
                    Modifier
                        .align(Alignment.BottomCenter)
                        .navigationBarsPadding()
                        .padding(horizontal = 24.dp, vertical = 28.dp)
                        .background(Color.Black.copy(alpha = 0.62f), RoundedCornerShape(18.dp))
                        .padding(horizontal = 14.dp, vertical = 10.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = Color.White,
            )
        }
    }
}

@Composable
private fun QrScannerCrosshair(modifier: Modifier = Modifier) {
    val path = remember { Path() }
    Canvas(modifier = modifier) {
        val crosshairWidth = size.minDimension * 0.6f
        val lineLength = crosshairWidth * 0.125f
        val topLeft = center - Offset(crosshairWidth / 2f, crosshairWidth / 2f)
        val topRight = center + Offset(crosshairWidth / 2f, -crosshairWidth / 2f)
        val bottomRight = center + Offset(crosshairWidth / 2f, crosshairWidth / 2f)
        val bottomLeft = center + Offset(-crosshairWidth / 2f, crosshairWidth / 2f)
        path.reset()
        path.moveTo(topLeft.x, topLeft.y + lineLength)
        path.lineTo(topLeft.x, topLeft.y)
        path.lineTo(topLeft.x + lineLength, topLeft.y)
        path.moveTo(topRight.x - lineLength, topRight.y)
        path.lineTo(topRight.x, topRight.y)
        path.lineTo(topRight.x, topRight.y + lineLength)
        path.moveTo(bottomRight.x, bottomRight.y - lineLength)
        path.lineTo(bottomRight.x, bottomRight.y)
        path.lineTo(bottomRight.x - lineLength, bottomRight.y)
        path.moveTo(bottomLeft.x + lineLength, bottomLeft.y)
        path.lineTo(bottomLeft.x, bottomLeft.y)
        path.lineTo(bottomLeft.x, bottomLeft.y - lineLength)
        drawPath(
            path = path,
            color = Color.White,
            style =
                Stroke(
                    width = 3.dp.toPx(),
                    pathEffect = PathEffect.cornerPathEffect(10.dp.toPx()),
                ),
        )
    }
}
