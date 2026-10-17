// CN Assassin Grand VII. Frontline: Koyanskaya / level-120 Gensai / Stheno.
require_profile("cn-assassin-grand-vii");

fn fight() {
    verify_opening();
    for skill in [1, 2, 3] { cast_skill(2, skill, true); }
    // Every ally-targeting skill uses the profile's support slot (2).
    for skill in [1, 2, 3] { cast_skill(1, skill, true); }
    cast_skill(2, 2, true);
    for skill in [1, 2, 3] { cast_skill(3, skill, true); }
    cast_master_skill(1, true);
    cast_master_skill(2, true);
    // Profile order is Stheno -> Gensai; Koyanskaya's NP is excluded.
    attack_nps([3, 2], true);
    let turns = 1;
    while wait_next_turn() {
        if turns >= 100 { fail("Battle exceeded 100 turns"); }
        verify_support();
        for servant in [2, 1, 3] {
            for skill in [1, 2, 3] { cast_skill(servant, skill, false); }
        }
        attack(false);
        turns += 1;
    }
    log(`Assassin VII battle finished in ${turns} turns`);
}

while enter_battle() {
    fight();
    finish_battle();
}
let progress = farm_progress();
log(`Farming stopped: ${progress.completed} battles, ${progress.apples_remaining} apples remaining`);
