use askama::Template;
use dropstf::{DropStats, PlayerTemplate, SmolStr, SteamId};
use iai::black_box;

const PLAYER: PlayerTemplate = PlayerTemplate {
    stats: DropStats {
        steam_id: SteamId::new(76561198024494988),
        name: SmolStr::new_inline("Icewind"),
        drops: 100,
        ubers: 50,
        games: 10,
        medic_time: 100,
        drops_rank: 1,
        dpu_rank: 2,
        dps_rank: 3,
        dpg_rank: 4,
    },
};

fn render_player() {
    let _ = black_box(black_box(PLAYER).render());
}

iai::main!(render_player);
