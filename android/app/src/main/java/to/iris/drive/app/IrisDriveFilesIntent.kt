package to.iris.drive.app

import android.content.Intent
import android.provider.DocumentsContract
import to.iris.drive.app.provider.IrisDriveDocumentStore

internal fun irisDriveFilesIntent(authority: String): Intent =
    Intent(Intent.ACTION_VIEW).setData(
        DocumentsContract.buildRootUri(authority, IrisDriveDocumentStore.ROOT_ID),
    )
