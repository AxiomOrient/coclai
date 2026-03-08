use super::*;

#[test]
fn method_constants_are_stable() {
    assert_eq!(methods::THREAD_START, "thread/start");
    assert_eq!(methods::TURN_INTERRUPT, "turn/interrupt");
}
