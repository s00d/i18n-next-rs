//! `FakeFetch` and the manual `Executor` (DESIGN 7.1): `ensure_async` without a runtime,
//! network or sleep. A fetch is a request that waits until the test answers it; the
//! executor polls the futures the test spawned, in the order the test chooses.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

#[derive(Default)]
struct Slot {
    answer: Option<Result<Vec<u8>, String>>,
    waker: Option<Waker>,
}

#[derive(Default)]
struct FetchState {
    /// Unanswered requests, oldest first.
    pending: Vec<(String, Arc<Mutex<Slot>>)>,
    /// Requests made per path, answered or not.
    counts: BTreeMap<String, usize>,
}

/// The fetch side of `ensure_async`, like `fakeHost` in nuxt-i18n-micro's loader model.
#[derive(Clone, Default)]
pub struct FakeFetch {
    state: Arc<Mutex<FetchState>>,
}

/// One request's future: ready once the test answers it.
pub struct Fetching {
    slot: Arc<Mutex<Slot>>,
}

impl Future for Fetching {
    type Output = Result<Vec<u8>, String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.slot.lock().unwrap();
        match slot.answer.take() {
            Some(a) => Poll::Ready(a),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl FakeFetch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The closure to hand to `ensure_async`: each call is one request.
    pub fn fetch(&self) -> impl Fn(String) -> Fetching + Clone + use<> {
        let state = self.state.clone();
        move |path| {
            let slot = Arc::new(Mutex::new(Slot::default()));
            let mut s = state.lock().unwrap();
            *s.counts.entry(path.clone()).or_default() += 1;
            s.pending.push((path, slot.clone()));
            Fetching { slot }
        }
    }

    /// Paths of the unanswered requests, oldest first.
    pub fn pending(&self) -> Vec<String> {
        let s = self.state.lock().unwrap();
        s.pending.iter().map(|(p, _)| p.clone()).collect()
    }

    /// Answers the pending request at `index` (of [`FakeFetch::pending`]) and wakes its
    /// future; returns its path.
    pub fn answer_at(&self, index: usize, answer: Result<&[u8], &str>) -> String {
        let (path, slot) = self.state.lock().unwrap().pending.remove(index);
        let mut slot = slot.lock().unwrap();
        slot.answer = Some(answer.map(<[u8]>::to_vec).map_err(str::to_owned));
        if let Some(w) = slot.waker.take() {
            w.wake();
        }
        path
    }

    /// Answers the oldest pending request for `path`.
    pub fn answer(&self, path: &str, answer: Result<&[u8], &str>) {
        let index = self
            .pending()
            .iter()
            .position(|p| p == path)
            .unwrap_or_else(|| panic!("no pending request for {path}"));
        self.answer_at(index, answer);
    }

    /// Requests made for `path` so far.
    pub fn count(&self, path: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .counts
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    /// Requests made so far, all paths.
    pub fn total(&self) -> usize {
        self.state.lock().unwrap().counts.values().sum()
    }
}

struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Task<'a> {
    future: Pin<Box<dyn Future<Output = ()> + 'a>>,
    woken: Arc<Flag>,
}

/// Polls spawned futures on the test's thread, in the test's order. A task is polled
/// only when it was woken (a new task starts woken); nothing runs between test steps.
#[derive(Default)]
pub struct Executor<'a> {
    tasks: Vec<Option<Task<'a>>>,
}

impl<'a> Executor<'a> {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Adds a task; its output lands in the returned cell when it completes.
    pub fn spawn<T: 'a>(&mut self, f: impl Future<Output = T> + 'a) -> Rc<RefCell<Option<T>>> {
        let out = Rc::new(RefCell::new(None));
        let cell = out.clone();
        self.tasks.push(Some(Task {
            future: Box::pin(async move {
                let v = f.await;
                *cell.borrow_mut() = Some(v);
            }),
            woken: Arc::new(Flag(AtomicBool::new(true))),
        }));
        out
    }

    /// Polls task `i` once if it was woken; whether it is done.
    pub fn poll(&mut self, i: usize) -> bool {
        let Some(task) = self.tasks.get_mut(i).and_then(Option::as_mut) else {
            return true;
        };
        if !task.woken.0.swap(false, Ordering::SeqCst) {
            return false;
        }
        let waker = Waker::from(task.woken.clone());
        if task
            .future
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_ready()
        {
            self.tasks[i] = None;
            return true;
        }
        false
    }

    /// Polls woken tasks in spawn order until none is woken.
    pub fn run_until_stalled(&mut self) {
        loop {
            let woken: Vec<usize> = (0..self.tasks.len())
                .filter(|&i| {
                    self.tasks[i]
                        .as_ref()
                        .is_some_and(|t| t.woken.0.load(Ordering::SeqCst))
                })
                .collect();
            if woken.is_empty() {
                return;
            }
            for i in woken {
                self.poll(i);
            }
        }
    }

    /// Tasks not finished yet.
    pub fn unfinished(&self) -> usize {
        self.tasks.iter().flatten().count()
    }
}

/// Successful lazy and remote loads of `i18n` so far, read from its `Debug`
/// (`I18n { .., loads: N, .. }`): no API for tests only.
pub fn loads(i18n: &i18n_next::I18n) -> u64 {
    let debug = format!("{i18n:?}");
    let at = debug.find("loads: ").expect("Debug shows loads") + "loads: ".len();
    debug[at..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
        .expect("a number after loads:")
}
