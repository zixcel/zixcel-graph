//! C4 private backend faults: no injection switches in released code.
use crate::{Graph, GraphChanges, GraphSpace, Node, PublicationIntent};
use zixcel_revision::{CommitOutcome, Failure, RevisionRef};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Point {
    Before,
    During,
    After,
}
thread_local! {static FAULT:std::cell::Cell<Option<Point>>=const {std::cell::Cell::new(None)};}
pub(super) fn checkpoint(point: Point) -> Result<(), crate::GraphError> {
    if point == Point::Before {
        PAUSE.with(|slot| {
            if let Some((entered, resume)) = slot.borrow_mut().take() {
                entered.send(()).unwrap();
                resume
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
            }
        });
    }
    if FAULT.get() == Some(point) {
        Err(if point == Point::After {
            Failure::DeliveryUnknown
        } else {
            Failure::Storage
        }
        .into())
    } else {
        Ok(())
    }
}

type Pause = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);
thread_local! {static PAUSE:std::cell::RefCell<Option<Pause>>=const {std::cell::RefCell::new(None)};}

#[test]
fn dependency_comparison_and_target_publication_share_writer_fence() {
    fn scenario(graph: &Graph) {
        let source = GraphSpace::new("atomic/source").unwrap();
        let target = GraphSpace::new("atomic/target").unwrap();
        let request = PublicationIntent {
            operation_id: "target".into(),
            expected_commit_revision: RevisionRef::default(),
        }
        .request("test", &0)
        .unwrap()
        .with_dependencies(vec![crate::GraphReadDependency {
            space: source.clone(),
            expected_revision: RevisionRef::default(),
        }])
        .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let target = scope.spawn(|| {
                PAUSE.with(|slot| *slot.borrow_mut() = Some((entered_tx, resume_rx)));
                graph
                    .publish(
                        &target,
                        &request,
                        |_| Ok(GraphChanges::default()),
                        || Ok(()),
                    )
                    .unwrap()
            });
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let source = scope.spawn(|| {
                started_tx.send(()).unwrap();
                let request = PublicationIntent {
                    operation_id: "source".into(),
                    expected_commit_revision: RevisionRef::default(),
                }
                .request("test", &0)
                .unwrap();
                let result = graph
                    .publish(
                        &source,
                        &request,
                        |_| Ok(GraphChanges::default()),
                        || Ok(()),
                    )
                    .unwrap();
                done_tx.send(()).unwrap();
                result
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert!(matches!(
                done_rx.recv_timeout(std::time::Duration::from_millis(30)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ));
            resume_tx.send(()).unwrap();
            assert!(matches!(
                target.join().unwrap().outcome,
                CommitOutcome::Committed(_)
            ));
            assert!(matches!(
                source.join().unwrap().outcome,
                CommitOutcome::Committed(_)
            ));
        });
    }
    scenario(&Graph::memory().unwrap());
    #[cfg(feature = "redb")]
    {
        let d = tempfile::tempdir().unwrap();
        scenario(&Graph::create(d.path().join("atomic.redb")).unwrap());
    }
}
fn scenario(graph: &Graph, path: Option<&std::path::Path>) {
    for point in [Point::Before, Point::During, Point::After] {
        let space = GraphSpace::new(format!("fault/{point:?}")).unwrap();
        let command = PublicationIntent {
            operation_id: "publish".into(),
            expected_commit_revision: RevisionRef::default(),
        }
        .request("fixture", &"input")
        .unwrap();
        let changes = || GraphChanges {
            nodes: vec![Node::new("prepared", ["fixture"]).unwrap()],
            ..Default::default()
        };
        // Failed/partially written preparation is either absent or exact Prepared.
        FAULT.set(Some(point));
        assert!(
            graph
                .prepare_publication(&space, &command, changes(), &[])
                .is_err()
        );
        FAULT.set(None);
        assert_eq!(
            graph.commit_revision(&space).unwrap(),
            RevisionRef::default()
        );
        assert!(graph.read(&space).unwrap().get_node("prepared").is_none());
        let prepared = graph
            .prepare_publication(&space, &command, changes(), &[])
            .unwrap();
        let mut sizes = vec![];
        for batch in 0..2 {
            for iteration in 0..100 {
                FAULT.set(Some(point));
                let result = graph.publish_prepared(&space, &prepared);
                FAULT.set(None);
                if point == Point::After && (batch > 0 || iteration > 0) {
                    assert!(matches!(
                        result.unwrap().outcome,
                        CommitOutcome::NoChange(_)
                    ));
                } else {
                    assert!(result.is_err());
                }
                let head = graph.commit_revision(&space).unwrap();
                assert_eq!(head.sequence, u64::from(point == Point::After));
                assert_eq!(
                    graph.read(&space).unwrap().get_node("prepared").is_some(),
                    point == Point::After
                );
            }
            sizes.push(path.map_or(0, |p| std::fs::metadata(p).unwrap().len()));
        }
        assert_eq!(
            sizes[0], sizes[1],
            "failed writes/replays did not plateau: {sizes:?}"
        );
        eprintln!("C4 {point:?} database allocation after 100/200 retries: {sizes:?}");
        let committed = graph.publish_prepared(&space, &prepared).unwrap();
        assert!(matches!(
            committed.outcome,
            CommitOutcome::Committed(_) | CommitOutcome::NoChange(_)
        ));
    }
}
#[test]
fn graph_backend_failure_boundaries_preserve_head_receipt_and_bounded_storage() {
    scenario(&Graph::memory().unwrap(), None);
    #[cfg(feature = "redb")]
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failures.redb");
        scenario(&Graph::create(&path).unwrap(), Some(&path));
        let graph = Graph::open_existing(&path).unwrap();
        for point in [Point::Before, Point::During, Point::After] {
            assert_eq!(
                graph
                    .commit_revision(&GraphSpace::new(format!("fault/{point:?}")).unwrap())
                    .unwrap()
                    .sequence,
                1
            );
        }
    }
}
