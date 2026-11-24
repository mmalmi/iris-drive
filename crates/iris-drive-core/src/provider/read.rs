//! Opening a provider file whose own blocks are not stored yet.

use std::path::Path;

use anyhow::{Context, Result};
use hashtree_core::Cid;
use hashtree_provider::{Item, ItemKind, ProviderFs};

/// Read a whole provider file, downloading its blocks first when needed.
///
/// A listing proves only that the directory blocks are local; the file's own
/// blocks can still be on their way when sync was interrupted or has not run.
/// Opening such a file downloads them from the configured file service, as
/// Drive does. When every block was already local, the read error stands.
pub async fn read_provider_file<P>(
    config_dir: &Path,
    provider: &P,
    path: &str,
) -> Result<(Item<String>, Vec<u8>)>
where
    P: ProviderFs<ItemId = String>,
{
    let item = provider.item(&path.to_owned()).await?;
    if item.kind == ItemKind::Directory {
        anyhow::bail!("cannot read directory: {path}");
    }
    let unread = match provider.read(&item.id, 0, item.size).await {
        Ok(bytes) => return Ok((item, bytes)),
        Err(error) => error,
    };
    let file = Cid::parse(&item.version).with_context(|| format!("parsing the CID of {path}"))?;
    let download = crate::network_sync::download_file_from_blossom(config_dir, &file)
        .await
        .with_context(|| {
            format!("{path} is not stored on this device ({unread}) and could not be downloaded")
        })?;
    if download.fetched == 0 {
        return Err(anyhow::Error::new(unread).context(format!("reading {path}")));
    }
    let bytes = provider
        .read(&item.id, 0, item.size)
        .await
        .with_context(|| format!("reading {path} after downloading it"))?;
    Ok((item, bytes))
}

#[cfg(test)]
mod tests;
