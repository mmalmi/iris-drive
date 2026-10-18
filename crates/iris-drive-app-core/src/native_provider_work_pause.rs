//! Deterministic scheduling at the boundary of real provider/network work.
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, mpsc};
use std::time::Duration;

struct Pause {
    entered: mpsc::Sender<()>,
    resume: mpsc::Receiver<()>,
}

static PAUSES: LazyLock<Mutex<BTreeMap<(String, &'static str), Pause>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(crate) struct WorkPause {
    key: (String, &'static str),
    entered: mpsc::Receiver<()>,
    resume: mpsc::Sender<()>,
}

impl WorkPause {
    pub(crate) fn new(data_dir: &str, kind: &'static str) -> Self {
        let (entered_tx, entered) = mpsc::channel();
        let (resume, resume_rx) = mpsc::channel();
        let key = (data_dir.to_owned(), kind);
        assert!(
            PAUSES
                .lock()
                .unwrap()
                .insert(
                    key.clone(),
                    Pause {
                        entered: entered_tx,
                        resume: resume_rx,
                    }
                )
                .is_none()
        );
        Self {
            key,
            entered,
            resume,
        }
    }

    pub(crate) fn await_work(&self) {
        self.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

impl Drop for WorkPause {
    fn drop(&mut self) {
        let _ = self.resume.send(());
        PAUSES.lock().unwrap().remove(&self.key);
    }
}

pub(crate) fn wait(data_dir: &str, kind: &'static str) {
    let pause = PAUSES.lock().unwrap().remove(&(data_dir.to_owned(), kind));
    if let Some(pause) = pause {
        pause.entered.send(()).unwrap();
        pause.resume.recv_timeout(Duration::from_secs(10)).unwrap();
    }
}
