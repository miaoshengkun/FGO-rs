// Read-only probe. Works with a live device or a single replay PNG.
capture();
for name in ["event", "quest", "rider_tab", "first_turn", "order_change", "master_ready", "master_used"] {
    let result = find(name);
    if result.found {
        log(`${name}: (${result.x}, ${result.y}), score=${result.score}`);
    }
}
let support = find("support120");
if support.found {
    log(`120-level friend: (${support.x}, ${support.y})`);
}
