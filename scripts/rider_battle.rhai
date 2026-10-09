// One CN Rider Grand battle, from an untouched first turn to the result screen.
// Enemy targeting follows the selected Party Profile; rider-v1 uses highest HP.
// Own team: Iskandar / Taigong / friend support; reserve slot 5: Reines.
verify_opening();
cast_skill(1, 1, true);
cast_skill(1, 2, true);
order_change();
for servant in [1, 2, 3] {
    for skill in [1, 2, 3] { cast_skill(servant, skill, true); }
}
cast_master_skill(1, true);
cast_master_skill(2, true);
attack(true);

let turns = 1;
while wait_next_turn() {
    if turns >= 100 { fail("Battle exceeded 100 turns"); }
    verify_support();
    for servant in [1, 2, 3] {
        for skill in [1, 2, 3] { cast_skill(servant, skill, false); }
    }
    cast_master_skill(1, false);
    cast_master_skill(2, false);
    attack(false);
    turns += 1;
}
log(`Battle finished in ${turns} turns; use rider_farm.rhai for settlement and repeat`);
