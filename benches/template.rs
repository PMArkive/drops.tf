use askama::Template;
use dropstf::{DropStats, PlayerTemplate};
use iai::black_box;

fn render_player() {
    let template = PlayerTemplate {
        stats: DropStats {
            steam_id: 76561198024494988.into(),
            name: "Icewind".into(),
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
    let _ = black_box(black_box(template).render());
}

iai::main!(render_player);
