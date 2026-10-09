// CN Saber Grand VII. Requires calibrated Saber templates; see docs/saber-vii.md.
require_profile("cn-saber-grand-vii");

fn follow_up() {
    verify_support();
    if attack_np(3, false) { return; }
    // Other servants' NPs take the entire turn before any ordinary skills.
    if attack_other_nps() { return; }
    for servant in [3, 1, 2] {
        for skill in [1, 2, 3] {
            if cast_skill(servant, skill, false) {
                // Recheck after each successful skill; stop spending skills
                // as soon as Iori can use his NP.
                if attack_np(3, false) { return; }
            }
        }
    }
    attack(false);
}

verify_opening();
cast_skill(1, 1, true);
cast_skill(1, 3, true);
order_change();
// Targeted skills use profile.support_field (3); party slot 5 is now field 1.
for servant in [1, 2, 3] {
    // Opening buffs can be temporarily sealed/disabled by the quest state;
    // skip those skills and continue with the remaining deterministic setup.
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
