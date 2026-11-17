use std::path::PathBuf;

use crate::screen_capture::system_audio_routing::*;

const OWN: u32 = 100;
const SINK: u32 = 1;
const OTHER: u32 = 2;
const SELF_PLAYER: u32 = 3;

fn port(node: u32, direction: AudioPortDirection, channel: &str) -> AudioGraphPort {
    AudioGraphPort { node, direction, channel: channel.into() }
}

fn graph() -> AudioGraph {
    let mut graph = AudioGraph::default();
    graph.add_port(10, port(SINK, AudioPortDirection::Input, "FL"));
    graph.add_port(11, port(SINK, AudioPortDirection::Input, "FR"));
    graph.add_port(12, port(SINK, AudioPortDirection::Output, "FL"));
    graph.add_player(OTHER);
    graph.add_port(20, port(OTHER, AudioPortDirection::Output, "FL"));
    graph.add_port(21, port(OTHER, AudioPortDirection::Output, "FR"));
    graph.add_player(SELF_PLAYER);
    graph.add_port(30, port(SELF_PLAYER, AudioPortDirection::Output, "FL"));
    graph
}

fn pairs(graph: &AudioGraph) -> Vec<(u32, u32)> {
    graph.wanted_links(SINK, OWN).into_iter().map(|link| (link.output_port, link.input_port)).collect()
}

#[test]
fn another_programs_channels_land_on_their_sides() {
    let mut graph = graph();
    graph.set_player_process(OTHER, Some(200));
    assert_eq!(pairs(&graph), vec![(20, 10), (21, 11)]);
}

#[test]
fn this_app_is_never_shared() {
    let mut graph = graph();
    graph.set_player_process(SELF_PLAYER, Some(OWN));
    assert!(pairs(&graph).is_empty());
}

#[test]
fn a_player_is_out_until_its_process_is_known() {
    let graph = graph();
    assert!(pairs(&graph).is_empty());
}

#[test]
fn a_player_with_no_process_is_shared() {
    let mut graph = graph();
    graph.set_player_process(OTHER, None);
    assert_eq!(pairs(&graph).len(), 2);
}

#[test]
fn mono_and_centre_go_to_both_sides() {
    for channel in ["MONO", "FC", "LFE"] {
        let mut graph = AudioGraph::default();
        graph.add_port(10, port(SINK, AudioPortDirection::Input, "FL"));
        graph.add_port(11, port(SINK, AudioPortDirection::Input, "FR"));
        graph.add_player(OTHER);
        graph.set_player_process(OTHER, Some(200));
        graph.add_port(20, port(OTHER, AudioPortDirection::Output, channel));
        assert_eq!(pairs(&graph), vec![(20, 10), (20, 11)], "{channel}");
    }
}

#[test]
fn removing_a_port_or_a_player_takes_its_links() {
    let mut graph = graph();
    graph.set_player_process(OTHER, Some(200));
    graph.remove(21);
    assert_eq!(pairs(&graph), vec![(20, 10)]);
    graph.remove(OTHER);
    assert!(pairs(&graph).is_empty());
}

#[test]
fn port_directions_parse() {
    assert_eq!(parse_port_direction("in"), Some(AudioPortDirection::Input));
    assert_eq!(parse_port_direction("out"), Some(AudioPortDirection::Output));
    assert_eq!(parse_port_direction("sideways"), None);
}

#[test]
fn the_socket_follows_the_client_library() {
    assert_eq!(pipewire_socket_path(None, None, Some("/run/user/1")), Some(PathBuf::from("/run/user/1/pipewire-0")));
    assert_eq!(pipewire_socket_path(None, Some("/pw"), Some("/run/user/1")), Some(PathBuf::from("/pw/pipewire-0")));
    assert_eq!(pipewire_socket_path(Some("other"), None, Some("/x")), Some(PathBuf::from("/x/other")));
    assert_eq!(pipewire_socket_path(Some("/abs/sock"), None, None), Some(PathBuf::from("/abs/sock")));
    assert_eq!(pipewire_socket_path(Some(""), Some(""), None), None);
}
