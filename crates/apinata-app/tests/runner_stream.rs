use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::runner_model::{RowState, RunState, RunnerModel};
use apinata_core::{StreamBound, StreamEnd};
use gpui_kit::AppContext as _;

#[gpui_kit::test]
fn the_collection_runner_stops_an_endless_stream_at_the_cli_limit(
    cx: &mut gpui_kit::TestAppContext,
) {
    http_ready(cx);
    let steps: Vec<Step> = (0..10_100).map(|i| event(&i.to_string())).collect();
    let server = sse(cx, Script::sse(steps));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let (w, r) = (ws.clone(), root.clone());
    let m = cx.new(|_| RunnerModel::new(w, r, ip("a.yml")));
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    m.read_with(cx, |m, _| {
        assert!(matches!(m.state(), RunState::Finished(_)));
        let RowState::Done(result) = &m.rows()[0].state else {
            panic!("not done")
        };
        let end = result.response.as_ref().and_then(|r| r.stream_end());
        assert_eq!(end, Some(&StreamEnd::Bound(StreamBound::Events)));
    });
}
