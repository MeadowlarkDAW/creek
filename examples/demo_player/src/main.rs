use rt_audio_disk_stream::ReadClient;
use rtrb::RingBuffer;

mod player;
mod ui;

pub enum GuiToPlayerMsg {
    PlayStream(ReadClient),
}

pub enum PlayerToGuiMsg {}

fn main() {
    let (to_gui_tx, from_player_rx) = RingBuffer::<PlayerToGuiMsg>::new(64).split();
    let (to_player_tx, from_gui_rx) = RingBuffer::<GuiToPlayerMsg>::new(64).split();

    let app = ui::DemoPlayerApp::new(to_player_tx, from_player_rx);
    let _cpal_stream = player::Player::new(to_gui_tx, from_gui_rx);

    eframe::run_native(Box::new(app));
}
