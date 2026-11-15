use super::*;

fn what(name: &str) -> egui::Id {
    egui::Id::unique(name)
}

#[test]
fn something_busy_closes_only_when_asked_twice() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    assert_eq!(closing.ask(what("tab"), false), Ask::First);
    assert!(!closing.press(what("tab"), false, true, now));
    assert_eq!(closing.ask(what("tab"), false), Ask::Again);
    assert!(closing.press(what("tab"), false, true, now));
    // Having closed, it waits for nothing.
    assert_eq!(closing.ask(what("tab"), false), Ask::First);
    assert_eq!(closing.lapse(now), None);
}

#[test]
fn something_with_nothing_to_lose_closes_at_once() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    assert_eq!(closing.ask(what("tab"), true), Ask::No);
    assert!(closing.press(what("tab"), true, true, now));
    assert_eq!(closing.lapse(now), None);
}

#[test]
fn asking_for_one_is_not_asking_for_another() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    assert!(!closing.press(what("one"), false, true, now));
    // A press meant for the first that lands on its neighbour closes neither.
    assert!(!closing.press(what("other"), false, true, now));
    assert_eq!(closing.ask(what("one"), false), Ask::First);
    assert_eq!(closing.ask(what("other"), false), Ask::Again);
    // Nor does it spare the neighbour the asking, were that one idle.
    assert_eq!(closing.ask(what("one"), true), Ask::No);
}

#[test]
fn the_wait_ends_when_the_pointer_leaves() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    closing.press(what("tab"), false, true, now);
    // The drawing it was asked in counts the pointer as by it.
    assert_eq!(closing.lapse(now), Some(Closing::TIME));
    closing.keep();
    let later = now + Duration::from_secs(1);
    assert_eq!(closing.lapse(later), Some(Closing::TIME - Duration::from_secs(1)));
    // Not kept since, it ends, and says so once.
    assert_eq!(closing.lapse(later), Some(Duration::ZERO));
    assert_eq!(closing.lapse(later), None);
    assert_eq!(closing.ask(what("tab"), false), Ask::First);
}

#[test]
fn the_wait_ends_with_time_wherever_the_pointer_is() {
    let (mut closing, now) = (Closing::default(), Instant::now());
    // Asked with a key, it does not go by the pointer.
    closing.press(what("pane"), false, false, now);
    assert_eq!(closing.lapse(now), Some(Closing::TIME));
    assert_eq!(closing.lapse(now), Some(Closing::TIME));
    assert_eq!(closing.lapse(now + Closing::TIME), Some(Duration::ZERO));
    assert_eq!(closing.ask(what("pane"), false), Ask::First);
    // Asked with the pointer and kept, it ends with time too.
    closing.press(what("tab"), false, true, now);
    closing.keep();
    assert_eq!(closing.lapse(now + Closing::TIME), Some(Duration::ZERO));
}
