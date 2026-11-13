use anyhow::Result;
use iris_drive_core::mirror;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Request {
    List,
    Add {
        source: PathBuf,
        destination: String,
        bookmark: String,
    },
    Run {
        id: String,
        destination: String,
        scheduled: bool,
        access_error: Option<String>,
    },
    Remove {
        destination: String,
    },
}

pub(super) fn dispatch(config: &Path, request: &str) -> Result<Value> {
    match serde_json::from_str(request)? {
        Request::List => Ok(json!({"folders":mirror::list(config)?})),
        Request::Add {
            source,
            destination,
            bookmark,
        } => {
            mirror::add(config, &source, Some(&destination), Some(bookmark))?;
            Ok(json!({}))
        }
        Request::Run {
            id,
            destination,
            scheduled,
            access_error,
        } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let report = rt.block_on(mirror::run_with_access(
                config,
                &destination,
                scheduled,
                Some(id.parse()?),
                access_error.as_deref(),
            ))?;
            Ok(json!({"report":report}))
        }
        Request::Remove { destination } => {
            mirror::remove(config, &destination)?;
            Ok(json!({}))
        }
    }
}
