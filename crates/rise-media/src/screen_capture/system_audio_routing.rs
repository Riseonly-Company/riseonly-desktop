use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const PLAYER_MEDIA_CLASS: &str = "Stream/Output/Audio";

const DEFAULT_REMOTE: &str = "pipewire-0";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioPortDirection {
    Input,
    Output,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AudioGraphPort {
    pub node: u32,
    pub direction: AudioPortDirection,
    pub channel: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct AudioLink {
    pub output_node: u32,
    pub output_port: u32,
    pub input_node: u32,
    pub input_port: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlayerProcess {
    Unread,
    Known(Option<u32>),
}

/// the part of the graph a share cares about
#[derive(Default)]
pub struct AudioGraph {
    players: BTreeMap<u32, PlayerProcess>,
    ports: BTreeMap<u32, AudioGraphPort>,
}

impl AudioGraph {
    pub fn add_player(&mut self, node: u32) {
        self.players.entry(node).or_insert(PlayerProcess::Unread);
    }

    pub fn set_player_process(&mut self, node: u32, process: Option<u32>) {
        if let Some(known) = self.players.get_mut(&node) {
            *known = PlayerProcess::Known(process);
        }
    }

    pub fn add_port(&mut self, id: u32, port: AudioGraphPort) {
        self.ports.insert(id, port);
    }

    /// pipewire ids are one space, so a removed global is a node or a port
    pub fn remove(&mut self, id: u32) {
        self.players.remove(&id);
        self.ports.remove(&id);
    }

    /// every other program output port into the inputs of sink_node, channel by channel
    pub fn wanted_links(&self, sink_node: u32, own_process: u32) -> BTreeSet<AudioLink> {
        let inputs: Vec<(u32, &AudioGraphPort)> = self
            .ports
            .iter()
            .filter(|(_, port)| port.node == sink_node && port.direction == AudioPortDirection::Input)
            .map(|(id, port)| (*id, port))
            .collect();
        let mut links = BTreeSet::new();
        for (output_port, output) in &self.ports {
            if output.direction != AudioPortDirection::Output {
                continue;
            }
            let shared = match self.players.get(&output.node) {
                Some(PlayerProcess::Known(process)) => *process != Some(own_process),
                Some(PlayerProcess::Unread) | None => false,
            };
            if !shared {
                continue;
            }
            for (input_port, input) in &inputs {
                if stereo_side_receives(&output.channel, &input.channel) {
                    links.insert(AudioLink {
                        output_node: output.node,
                        output_port: *output_port,
                        input_node: sink_node,
                        input_port: *input_port,
                    });
                }
            }
        }
        links
    }
}

fn stereo_side_receives(output_channel: &str, input_channel: &str) -> bool {
    match output_channel {
        "FL" | "RL" | "SL" | "FLC" => input_channel == "FL",
        "FR" | "RR" | "SR" | "FRC" => input_channel == "FR",
        _ => true,
    }
}

pub fn parse_graph_id(text: &str) -> Option<u32> {
    text.parse().ok()
}

pub fn parse_port_direction(text: &str) -> Option<AudioPortDirection> {
    match text {
        "in" => Some(AudioPortDirection::Input),
        "out" => Some(AudioPortDirection::Output),
        _ => None,
    }
}

/// where the client library would connect, by the variables it reads
pub fn pipewire_socket_path(remote: Option<&str>, runtime_dir: Option<&str>, xdg_runtime_dir: Option<&str>) -> Option<PathBuf> {
    let name = remote.filter(|name| !name.is_empty()).unwrap_or(DEFAULT_REMOTE);
    if name.starts_with('/') {
        return Some(PathBuf::from(name));
    }
    let directory = runtime_dir.filter(|dir| !dir.is_empty()).or(xdg_runtime_dir.filter(|dir| !dir.is_empty()))?;
    Some(PathBuf::from(directory).join(name))
}
