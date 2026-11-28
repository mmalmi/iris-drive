//! Opening a provider file whose own blocks are not stored yet.

use std::path::Path;

use anyhow::{Context, Result};
use hashtree_core::{Cid, Store};
use hashtree_provider::{HashTreeProviderFs, Item, ItemKind, ProviderError, ProviderFs};

/// Read a whole provider file into memory, downloading its blocks first when needed.
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
    download_missing_provider_file(config_dir, path, &item.version, unread).await?;
    let bytes = provider
        .read(&item.id, 0, item.size)
        .await
        .with_context(|| format!("reading {path} after downloading it"))?;
    Ok((item, bytes))
}

/// Stream a provider file chunk by chunk to `output`, downloading missing
/// blocks from the configured file service first when needed.
///
/// Avoids holding large videos in memory on iOS/Android extensions and desktop
/// provider bridges.
pub async fn read_provider_file_to_path<S>(
    config_dir: &Path,
    provider: &HashTreeProviderFs<S>,
    path: &str,
    output: &Path,
) -> Result<Item<String>>
where
    S: Store + 'static,
{
    let unread = match provider.read_file_to_path(path, output).await {
        Ok(item) => return Ok(item),
        Err(ProviderError::IsDir) => anyhow::bail!("cannot read directory: {path}"),
        Err(error @ ProviderError::NotFound) => error,
        Err(error) => return Err(error.into()),
    };
    let item = provider.item(&path.to_owned()).await?;
    download_missing_provider_file(config_dir, path, &item.version, unread).await?;
    provider
        .read_file_to_path(&item.id, output)
        .await
        .with_context(|| format!("reading {path} after downloading it"))
}

async fn download_missing_provider_file(
    config_dir: &Path,
    path: &str,
    version: &str,
    unread: ProviderError,
) -> Result<()> {
    let file = Cid::parse(version).with_context(|| format!("parsing the CID of {path}"))?;
    let download = crate::network_sync::download_file_from_blossom(config_dir, &file)
        .await
        .with_context(|| {
            format!("{path} is not stored on this device ({unread}) and could not be downloaded")
        })?;
    if download.fetched == 0 {
        return Err(anyhow::Error::new(unread).context(format!("reading {path}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
