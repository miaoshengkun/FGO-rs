// CN Saber Grand VII, including support selection and settlement.
// Requires calibrated Saber templates; see docs/saber-vii.md.
require_profile("cn-saber-grand-vii");

fn follow_up() {
    verify_support();
    if attack_np(3, false) { return; }
    if attack_other_nps() { return; }
    for servant in [3, 1, 2] {
        for skill in [1, 2, 3] {
            if cast_skill(servant, skill, false) {
                if attack_np(3, false) { return; }
            }
        }
    }
    attack(false);
}

fn fight() {
    verify_opening();
    cast_skill(1, 1, true);
    cast_skill(1, 3, true);
    order_change();
    for servant in [1, 2, 3] {
        // Quest-side sealing can disable a non-essential opening skill;
        // skip disabled skills while preserving strict unknown-state stops.
        for skill in [1, 2, 3] { cast_skill(servant, skill, false); }
    }
    cast_master_skill(1, true);
    attack_np(3, true);
    let turns = 1;
    while wait_next_turn() {
        if turns >= 100 { fail("Battle exceeded 100 turns"); }
        follow_up();
        turns += 1;
    }
    log(`Saber VII battle finished in ${turns} turns`);
}

while enter_battle() {
    fight();
    finish_battle();
}
let progress = farm_progress();
log(`Farming stopped: ${progress.completed} battles, ${progress.apples_remaining} apples remaining`);
