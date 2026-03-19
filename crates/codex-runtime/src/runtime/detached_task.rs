use std::future::Future;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetachedTaskRuntime {
    CurrentTokio,
    HelperThreadTokio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DetachedTaskPlan {
    pub(crate) name: &'static str,
    pub(crate) runtime: DetachedTaskRuntime,
}

pub(crate) fn detached_task_runtime(has_current_runtime: bool) -> DetachedTaskRuntime {
    if has_current_runtime {
        DetachedTaskRuntime::CurrentTokio
    } else {
        DetachedTaskRuntime::HelperThreadTokio
    }
}

pub(crate) fn detached_task_plan(
    name: &'static str,
    has_current_runtime: bool,
) -> DetachedTaskPlan {
    DetachedTaskPlan {
        name,
        runtime: detached_task_runtime(has_current_runtime),
    }
}

pub(crate) fn current_detached_task_plan(name: &'static str) -> DetachedTaskPlan {
    detached_task_plan(name, tokio::runtime::Handle::try_current().is_ok())
}

pub(crate) fn spawn_detached_task<F, C>(
    future: F,
    plan: DetachedTaskPlan,
    on_helper_runtime_init_failed: C,
) where
    F: Future<Output = ()> + Send + 'static,
    C: FnOnce() + Send + 'static,
{
    match plan.runtime {
        DetachedTaskRuntime::CurrentTokio => {
            tokio::spawn(future);
        }
        DetachedTaskRuntime::HelperThreadTokio => {
            std::thread::spawn(move || {
                if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    rt.block_on(future);
                } else {
                    tracing::warn!(
                        task = plan.name,
                        "failed to initialize helper tokio runtime for detached task"
                    );
                    on_helper_runtime_init_failed();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn detached_task_runtime_is_data_first() {
        assert_eq!(
            detached_task_runtime(true),
            DetachedTaskRuntime::CurrentTokio
        );
        assert_eq!(
            detached_task_runtime(false),
            DetachedTaskRuntime::HelperThreadTokio
        );
    }

    #[test]
    fn detached_task_plan_keeps_name_and_runtime_together() {
        assert_eq!(
            detached_task_plan("cleanup", true),
            DetachedTaskPlan {
                name: "cleanup",
                runtime: DetachedTaskRuntime::CurrentTokio,
            }
        );
        assert_eq!(
            detached_task_plan("cleanup", false),
            DetachedTaskPlan {
                name: "cleanup",
                runtime: DetachedTaskRuntime::HelperThreadTokio,
            }
        );
    }

    #[test]
    fn helper_thread_runtime_executes_future() {
        let (tx, rx) = mpsc::channel();
        spawn_detached_task(
            async move {
                tx.send("done").expect("send completion");
            },
            DetachedTaskPlan {
                name: "helper-thread-test",
                runtime: DetachedTaskRuntime::HelperThreadTokio,
            },
            || panic!("helper runtime should initialize"),
        );

        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1))
                .expect("receive completion"),
            "done"
        );
    }
}
