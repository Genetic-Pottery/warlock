use std::ops::ControlFlow;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{Lost, Once, Port, Stream, Workers, settled};

const AT_MOST: Duration = Duration::from_secs(10);

const BOTH: [Workers; 2] = [Workers::Threaded, Workers::Inline];

fn landing<T>(once: &Once<T>) -> Result<T, Lost> {
    let waited = Instant::now();
    loop {
        if let Some(landed) = once.landed() {
            return landed;
        }
        assert!(waited.elapsed() < AT_MOST, "the work never landed");
    }
}

fn ending<E>(stream: &Stream<E>, seen: &mut Vec<E>) -> Result<(), Lost> {
    let waited = Instant::now();
    loop {
        let drained = stream.drained(|event| {
            seen.push(event);
            ControlFlow::Continue(())
        });
        match drained {
            Some(ended) => return ended,
            None => assert!(waited.elapsed() < AT_MOST, "the stream never ended"),
        }
    }
}

#[test]
fn work_that_says_one_thing_lands_it() {
    for workers in BOTH {
        assert_eq!(landing(&workers.once(|| 7)), Ok(7), "{workers:?}");
    }
}

#[test]
fn work_that_panics_is_lost_rather_than_silent() {
    for workers in BOTH {
        let once: Once<u8> = workers.once(|| panic!("a worker that says nothing"));
        assert_eq!(landing(&once), Err(Lost), "{workers:?}");
    }
}

#[test]
fn inline_work_has_landed_by_the_time_the_spawn_returns() {
    assert_eq!(Workers::Inline.once(|| "said").landed(), Some(Ok("said")));

    let stream = Workers::Inline.stream(|port: Port<u8>| {
        move || {
            port.send(1);
            port.send(2);
        }
    });
    let mut seen = Vec::new();
    assert_eq!(
        stream.drained(|event| {
            seen.push(event);
            ControlFlow::<()>::Continue(())
        }),
        Some(Err(Lost))
    );
    assert_eq!(seen, [1, 2]);
}

#[test]
fn a_stream_ends_at_the_event_the_caller_breaks_on() {
    for workers in BOTH {
        let stream = workers.stream(|port: Port<u8>| {
            move || {
                for event in 1..=3 {
                    port.send(event);
                }
            }
        });
        let waited = Instant::now();
        let mut seen = Vec::new();
        let ended = loop {
            let drained = stream.drained(|event| {
                seen.push(event);
                if event == 3 {
                    ControlFlow::Break("over")
                } else {
                    ControlFlow::Continue(())
                }
            });
            if let Some(ended) = drained {
                break ended;
            }
            assert!(waited.elapsed() < AT_MOST, "the stream never ended");
        };
        assert_eq!(ended, Ok("over"), "{workers:?}");
        assert_eq!(seen, [1, 2, 3], "{workers:?}");
    }
}

#[test]
fn a_stream_that_closes_before_its_ending_is_lost_after_everything_it_said() {
    for workers in BOTH {
        let stream = workers.stream(|port: Port<u8>| {
            let wired = port.clone();
            move || {
                wired.send(1);
                port.send(2);
            }
        });
        let mut seen = Vec::new();
        assert_eq!(ending(&stream, &mut seen), Err(Lost), "{workers:?}");
        assert_eq!(seen, [1, 2], "{workers:?}");
    }
}

#[test]
fn a_port_whose_receiver_is_gone_sends_into_nothing() {
    let (sender, received) = mpsc::channel();
    drop(received);
    Port::from(sender).send(1);
}

#[test]
fn a_slot_is_left_alone_until_its_work_has_something_to_say() {
    let (sender, received) = mpsc::channel();
    let mut slot = Some(Once::from(received));

    assert!(settled(&mut slot, |once| once.landed()).is_none());
    assert!(slot.is_some(), "a quiet worker was taken down");

    sender.send(5).expect("the slot still holds the receiver");
    let (_, landed) = settled(&mut slot, |once| once.landed()).expect("the work said something");

    assert_eq!(landed, Ok(5));
    assert!(slot.is_none(), "the slot outlived the work it held");
}

#[test]
fn an_empty_slot_settles_nothing() {
    let mut slot: Option<Once<u8>> = None;
    assert!(settled(&mut slot, |once| once.landed()).is_none());
}
