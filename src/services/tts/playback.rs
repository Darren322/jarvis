use std::{fs::File, path::Path};

use rodio::{
    Player,
    stream::{DeviceSinkBuilder, DeviceSinkError, MixerDeviceSink, play},
};

pub struct AudioPlayer {
    device: MixerDeviceSink,
    player: Option<Player>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self, DeviceSinkError> {
        let device = DeviceSinkBuilder::open_default_sink()?;

        Ok(Self {
            device,
            player: None,
        })
    }

    pub fn play(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        self.stop();

        let file = File::open(path)?;
        let player = play(self.device.mixer(), file)?;

        self.player = Some(player);

        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }

    pub fn is_playing(&self) -> bool {
        self.player.as_ref().is_some_and(|player| !player.empty())
    }
}

