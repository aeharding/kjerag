//! One file-preparation worker, with one replaceable queued request. A network
//! read can block that worker, never App::init or the UI event loop. Closing or
//! choosing another file revokes publication without joining filesystem IO.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, Weak};

use cosmic::iced::futures::channel::oneshot;
use kjerag_render::{Fallible, Framing};

pub(crate) struct OpenRequest {
    pub(crate) path: PathBuf,
    pub(crate) alongside: Vec<PathBuf>,
    pub(crate) framing: Option<Framing>,
    pub(crate) pasted: bool,
}

type Prepare<T> = Box<dyn Fn(&Path, &[PathBuf]) -> Fallible<T> + Send>;
type Reply<T> = Arc<Mutex<Option<Fallible<T>>>>;

struct Work<T> {
    path: PathBuf,
    alongside: Vec<PathBuf>,
    result: Weak<Mutex<Option<Fallible<T>>>>,
    ready: oneshot::Sender<()>,
}

struct State<T> {
    queued: Option<Work<T>>,
    closed: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

struct Pending<T> {
    id: u64,
    request: OpenRequest,
    reply: Reply<T>,
}

pub(crate) struct Opener<T> {
    shared: Arc<Shared<T>>,
    prepare: Option<Prepare<T>>,
    pending: Option<Pending<T>>,
    next: u64,
}

impl<T: Send + 'static> Opener<T> {
    pub(crate) fn new(prepare: impl Fn(&Path, &[PathBuf]) -> Fallible<T> + Send + 'static) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    queued: None,
                    closed: false,
                }),
                changed: Condvar::new(),
            }),
            prepare: Some(Box::new(prepare)),
            pending: None,
            next: 0,
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn request(
        &mut self,
        request: OpenRequest,
    ) -> Fallible<impl Future<Output = u64> + Send + 'static> {
        if let Some(prepare) = self.prepare.take() {
            let shared = self.shared.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("kjerag-open".into())
                .spawn(move || run(shared, prepare))
            {
                self.shared.state.lock().unwrap().closed = true;
                return Err(error.into());
            }
        }
        let mut state = self.shared.state.lock().unwrap();
        if state.closed {
            return Err("file preparation worker stopped before opening the file".into());
        }
        self.next += 1;
        let id = self.next;
        let (sender, receiver) = oneshot::channel();
        let reply = Arc::new(Mutex::new(None));
        state.queued = Some(Work {
            path: request.path.clone(),
            alongside: request.alongside.clone(),
            result: Arc::downgrade(&reply),
            ready: sender,
        });
        self.pending = Some(Pending {
            id,
            request,
            reply: reply.clone(),
        });
        self.shared.changed.notify_one();
        let reply = Arc::downgrade(&reply);
        Ok(async move {
            if receiver.await.is_err()
                && let Some(reply) = reply.upgrade()
            {
                *reply.lock().unwrap() = Some(Err(
                    "file preparation worker stopped before returning a result".into(),
                ));
            }
            id
        })
    }

    /// Only the current request may install a Scene, apply a view, remember a
    /// file, or surface an error. Delayed messages for older choices are inert.
    pub(crate) fn take(&mut self, id: u64) -> Option<(OpenRequest, Fallible<T>)> {
        let pending = self.pending.as_ref().filter(|pending| pending.id == id)?;
        let result = pending.reply.lock().unwrap().take()?;
        let pending = self.pending.take().unwrap();
        Some((pending.request, result))
    }

    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.shared.state.lock().unwrap().queued = None;
    }
}

impl<T> Drop for Opener<T> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.closed = true;
        state.queued = None;
        self.shared.changed.notify_one();
        // No join: the worker owns a pending read until the filesystem returns.
    }
}

/// Also closes queued senders if preparation panics, so a dead worker cannot
/// strand the current request or silently accept later work forever.
struct WorkerExit<T>(Arc<Shared<T>>);

impl<T> Drop for WorkerExit<T> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.closed = true;
        state.queued = None;
    }
}

fn run<T>(shared: Arc<Shared<T>>, prepare: Prepare<T>) {
    let _exit = WorkerExit(shared.clone());
    loop {
        let work = {
            let mut state = shared.state.lock().unwrap();
            while state.queued.is_none() && !state.closed {
                state = shared.changed.wait(state).unwrap();
            }
            if state.closed {
                return;
            }
            state.queued.take().unwrap()
        };
        let result = prepare(&work.path, &work.alongside);
        if let Some(reply) = work.result.upgrade() {
            *reply.lock().unwrap() = Some(result);
        }
        // A stale future carries only an ID, never a prepared Reader. Delaying
        // UI messages cannot accumulate captures from superseded choices.
        let _ = work.ready.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::futures::executor::block_on;
    use std::sync::mpsc;
    use std::time::Duration;

    fn request(path: &str) -> OpenRequest {
        OpenRequest {
            path: path.into(),
            alongside: Vec::new(),
            framing: None,
            pasted: false,
        }
    }

    #[test]
    fn blocked_open_returns_immediately_and_only_latest_queued_choice_runs() {
        let (started, starts) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let mut opener = Opener::new(move |path, _| {
            started.send(path.to_owned()).unwrap();
            if path == Path::new("first") {
                releases.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            Ok(path.to_owned())
        });
        let first = opener.request(request("first")).unwrap();
        assert_eq!(
            starts.recv_timeout(Duration::from_secs(2)).unwrap(),
            Path::new("first")
        );
        let superseded = opener.request(request("second")).unwrap();
        let latest = opener.request(request("third")).unwrap();
        assert!(opener.is_pending());
        assert!(opener.take(block_on(superseded)).is_none());
        release.send(()).unwrap();
        assert!(opener.take(block_on(first)).is_none());
        let id = block_on(latest);
        assert_eq!(
            starts.recv_timeout(Duration::from_secs(2)).unwrap(),
            Path::new("third")
        );
        let (request, result) = opener.take(id).unwrap();
        assert_eq!(request.path, Path::new("third"));
        assert_eq!(result.unwrap(), Path::new("third"));
        assert!(!opener.is_pending());
        assert!(opener.take(id).is_none());
        assert!(starts.try_recv().is_err());
    }

    #[test]
    fn closing_revokes_an_in_flight_result_and_a_queued_choice() {
        let (started, starts) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let mut opener = Opener::new(move |_, _| {
            started.send(()).unwrap();
            releases.recv_timeout(Duration::from_secs(3)).unwrap();
            Err::<(), _>("backend IO error".into())
        });
        let first = opener.request(request("first")).unwrap();
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        let queued = opener.request(request("queued")).unwrap();
        opener.cancel();
        assert!(!opener.is_pending());
        assert!(opener.take(block_on(queued)).is_none());
        release.send(()).unwrap();
        assert!(opener.take(block_on(first)).is_none());
        assert!(starts.try_recv().is_err());
    }

    #[test]
    fn dropping_the_shell_does_not_join_blocked_preparation() {
        let (started, starts) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let mut opener = Opener::new(move |_, _| {
            started.send(()).unwrap();
            releases.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(())
        });
        let pending = opener.request(request("first")).unwrap();
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(opener);
        // Releasing only after drop would deadlock a destructor that joined.
        release.send(()).unwrap();
        block_on(pending);
    }

    #[test]
    fn a_superseded_result_is_released_without_polling_its_ui_future() {
        struct Prepared(PathBuf, mpsc::Sender<PathBuf>);
        impl Drop for Prepared {
            fn drop(&mut self) {
                let _ = self.1.send(self.0.clone());
            }
        }
        let (started, starts) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let (dropped, drops) = mpsc::channel();
        let mut opener = Opener::new(move |path, _| {
            started.send(()).unwrap();
            if path == Path::new("first") {
                releases.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            Ok(Prepared(path.to_owned(), dropped.clone()))
        });
        let _unpolled = opener.request(request("first")).unwrap();
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        let latest = opener.request(request("latest")).unwrap();
        release.send(()).unwrap();
        assert_eq!(
            drops.recv_timeout(Duration::from_secs(2)).unwrap(),
            Path::new("first")
        );
        let (_, prepared) = opener.take(block_on(latest)).unwrap();
        drop(prepared);
        assert_eq!(
            drops.recv_timeout(Duration::from_secs(2)).unwrap(),
            Path::new("latest")
        );
    }

    #[test]
    fn worker_exit_resolves_active_and_queued_requests_and_refuses_new_work() {
        let (started, starts) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let mut opener = Opener::new(move |_, _| -> Fallible<()> {
            started.send(()).unwrap();
            releases.recv_timeout(Duration::from_secs(3)).unwrap();
            panic!("preparation failure");
        });
        let first = opener.request(request("first")).unwrap();
        starts.recv_timeout(Duration::from_secs(2)).unwrap();
        let latest = opener.request(request("latest")).unwrap();
        release.send(()).unwrap();
        assert!(opener.take(block_on(first)).is_none());
        let (_, result) = opener.take(block_on(latest)).unwrap();
        assert_eq!(
            result.unwrap_err().to_string(),
            "file preparation worker stopped before returning a result"
        );
        assert!(opener.request(request("next")).is_err());
    }
}
