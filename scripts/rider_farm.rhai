// Rider Grand VII farming from the event list, support list, formation or repeat prompt.
// Limits are supplied by --battles, --apples, --apple-kind and --max-seconds.
fn fight() {
    verify_opening();
    cast_skill(1,1,true);
    cast_skill(1,2,true);
    order_change();
    for servant in [1,2,3] {
        for skill in [1,2,3] {cast_skill(servant,skill,true);}
    }
    cast_master_skill(1,true);
    cast_master_skill(2,true);
    attack(true);
    let turns=1;
    while wait_next_turn() {
        if turns>=100 {fail("Battle exceeded 100 turns");}
        verify_support();
        for servant in [1,2,3] {
            for skill in [1,2,3] {cast_skill(servant,skill,false);}
        }
        cast_master_skill(1,false);
        cast_master_skill(2,false);
        attack(false);
        turns+=1;
    }
    log(`Battle finished in ${turns} turns`);
}

while enter_battle() {
    fight();
    finish_battle();
}
let progress=farm_progress();
log(`Farming stopped: ${progress.completed} battles, ${progress.apples_remaining} apples remaining`);
