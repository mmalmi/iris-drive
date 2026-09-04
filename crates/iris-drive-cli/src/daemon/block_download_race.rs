#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BlockDownloadTransport {
    Fips,
    Blossom,
}

struct BlockDownloadOutcome {
    transport: BlockDownloadTransport,
    report: DownloadReport,
    prior_errors: Vec<BlockDownloadError>,
}

#[derive(Debug)]
struct BlockDownloadError {
    transport: BlockDownloadTransport,
    message: String,
}

impl BlockDownloadError {
    fn summary(&self) -> String {
        let transport = match self.transport {
            BlockDownloadTransport::Fips => "fips",
            BlockDownloadTransport::Blossom => "blossom",
        };
        format!("{transport}: {}", self.message)
    }
}

fn log_block_download_errors(
    root_cid: &str,
    connected_peers: &[String],
    errors: &[BlockDownloadError],
) {
    for error in errors {
        match error.transport {
            BlockDownloadTransport::Fips => println!(
                "{}",
                json!({
                    "event": "fips_download_error",
                    "root_cid": root_cid,
                    "error": error.message,
                    "connected_peers": connected_peers,
                })
            ),
            BlockDownloadTransport::Blossom => println!(
                "{}",
                json!({
                    "event": "blossom_download_error",
                    "root_cid": root_cid,
                    "error": error.message,
                })
            ),
        }
    }
}

type BlockDownloadFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<DownloadReport, String>> + Send + 'a>,
>;

async fn first_successful_block_download(
    fips: Option<BlockDownloadFuture<'_>>,
    blossom: Option<BlockDownloadFuture<'_>>,
) -> std::result::Result<BlockDownloadOutcome, Vec<BlockDownloadError>> {
    match (fips, blossom) {
        (Some(mut fips), Some(mut blossom)) => {
            tokio::select! {
                result = &mut fips => finish_or_await_block_download(
                    BlockDownloadTransport::Fips,
                    result,
                    BlockDownloadTransport::Blossom,
                    blossom,
                ).await,
                result = &mut blossom => finish_or_await_block_download(
                    BlockDownloadTransport::Blossom,
                    result,
                    BlockDownloadTransport::Fips,
                    fips,
                ).await,
            }
        }
        (Some(fips), None) => single_block_download(BlockDownloadTransport::Fips, fips).await,
        (None, Some(blossom)) => {
            single_block_download(BlockDownloadTransport::Blossom, blossom).await
        }
        (None, None) => Err(Vec::new()),
    }
}

async fn finish_or_await_block_download(
    first_transport: BlockDownloadTransport,
    first: Result<DownloadReport, String>,
    second_transport: BlockDownloadTransport,
    second: BlockDownloadFuture<'_>,
) -> std::result::Result<BlockDownloadOutcome, Vec<BlockDownloadError>> {
    match first {
        Ok(report) => Ok(BlockDownloadOutcome {
            transport: first_transport,
            report,
            prior_errors: Vec::new(),
        }),
        Err(first_error) => match second.await {
            Ok(report) => Ok(BlockDownloadOutcome {
                transport: second_transport,
                report,
                prior_errors: vec![BlockDownloadError {
                    transport: first_transport,
                    message: first_error,
                }],
            }),
            Err(second_error) => Err(vec![
                BlockDownloadError {
                    transport: first_transport,
                    message: first_error,
                },
                BlockDownloadError {
                    transport: second_transport,
                    message: second_error,
                },
            ]),
        },
    }
}

async fn single_block_download(
    transport: BlockDownloadTransport,
    download: BlockDownloadFuture<'_>,
) -> std::result::Result<BlockDownloadOutcome, Vec<BlockDownloadError>> {
    download
        .await
        .map(|report| BlockDownloadOutcome {
            transport,
            report,
            prior_errors: Vec::new(),
        })
        .map_err(|message| vec![BlockDownloadError { transport, message }])
}
