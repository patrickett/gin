mod support;

use support::expect_flaws_with;

const ARENA: &str = r#"
Byte is in 0 to 255
Byte has
    finish(self):
        return
replace(previous Byte, next Byte) Byte: return next
read(value ref Byte) Byte: return deref value
Slot has
    value Byte
    reclaim(self):
        return
Arena has
    left Slot
    right Slot
    used Byte
    release(self):
        Arena(left, right, used) := self
        left.reclaim
        right.reclaim
        return
make Arena: Arena(left: Slot(value: 7), right: Slot(value: 19), used: 0)
Block has
    value ref Byte
    finish(self):
        return
reserve_left(arena mut Arena) Block:
    arena.used:: replace(arena.used, 1)
    return Block(value: arena.left.value)
reserve_right(arena mut Arena) Block:
    arena.used:: replace(arena.used, 2)
    return Block(value: arena.right.value)
"#;

#[test]
fn bookkeeping_preserves_a_direct_borrow_of_nested_storage() {
    expect_flaws_with(
        ARENA,
        r#"
run Byte:
    arena: make
    ref saved Byte: arena.left.value
    arena.used:: replace(arena.used, 1)
    result: read(saved)
    arena.release
    return result
"#,
        &[],
    );
}

#[test]
fn reserving_another_slot_preserves_the_first_returned_block() {
    expect_flaws_with(
        ARENA,
        r#"
run Byte:
    arena: make
    first: reserve_left(arena)
    second: reserve_right(arena)
    read(second.value)
    result: read(first.value)
    arena.release
    return result
"#,
        &[],
    );
}

#[test]
fn replacing_another_slot_preserves_the_first_returned_block() {
    expect_flaws_with(
        ARENA,
        r#"
replace_slot(previous Slot, next Slot) Slot:
    previous.reclaim
    return next
run Byte:
    arena: make
    first: reserve_left(arena)
    arena.right:: replace_slot(arena.right, Slot(value: 23))
    result: read(first.value)
    arena.release
    return result
"#,
        &[],
    );
}

#[test]
fn replacing_a_borrowed_slot_invalidates_the_returned_block() {
    expect_flaws_with(
        ARENA,
        r#"
replace_slot(previous Slot, next Slot) Slot:
    previous.reclaim
    return next
run Byte:
    arena: make
    first: reserve_left(arena)
    arena.left:: replace_slot(arena.left, Slot(value: 23))
    result: read(first.value)
    arena.release
    return result
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn reclaiming_another_slot_preserves_the_first_returned_block() {
    expect_flaws_with(
        ARENA,
        r#"
run Byte:
    arena: make
    first: reserve_left(arena)
    arena.right.reclaim
    result: read(first.value)
    arena.left.reclaim
    return result
"#,
        &[],
    );
}

#[test]
fn reclaiming_a_borrowed_slot_invalidates_the_returned_block() {
    expect_flaws_with(
        ARENA,
        r#"
run Byte:
    arena: make
    first: reserve_left(arena)
    arena.left.reclaim
    arena.right.reclaim
    return read(first.value)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn releasing_an_arena_invalidates_its_returned_blocks() {
    expect_flaws_with(
        ARENA,
        r#"
run Byte:
    arena: make
    first: reserve_left(arena)
    arena.release
    return read(first.value)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn unread_block_can_finish_after_arena_release() {
    expect_flaws_with(
        ARENA,
        r#"
run:
    arena: make
    first: reserve_left(arena)
    arena.release
    first.finish
    return
"#,
        &[],
    );
}

#[test]
fn moving_a_block_preserves_its_storage_dependency() {
    expect_flaws_with(
        ARENA,
        r#"
move_block(block own Block) Block: return block
run Byte:
    arena: make
    first: reserve_left(arena)
    moved: move_block(first)
    arena.release
    return read(moved.value)
"#,
        &["type-use-of-invalidated-reference"],
    );
}

#[test]
fn wrapped_blocks_preserve_borrows_across_bookkeeping() {
    expect_flaws_with(
        ARENA,
        r#"
Lease has
    block Block
    finish(self):
        Lease(block) := self
        block.finish
        return
run Byte:
    arena: make
    lease: Lease(block: reserve_left(arena))
    reserve_right(arena).finish
    result: read(lease.block.value)
    lease.finish
    arena.release
    return result
"#,
        &[],
    );
}

#[test]
fn derived_group_accepts_a_slot_from_the_selected_arena() {
    expect_flaws_with(
        ARENA,
        r#"
write(arena ref{storage} Arena, slot mut{storage.left} Slot):
    slot.value:: replace(slot.value, 23)
    return
run:
    arena: make
    write(arena, arena.left)
    arena.release
    return
"#,
        &[],
    );
}

#[test]
fn derived_group_rejects_a_slot_from_another_arena() {
    expect_flaws_with(
        ARENA,
        r#"
write(arena ref{storage} Arena, slot mut{storage.left} Slot):
    slot.value:: replace(slot.value, 23)
    return
run:
    first: make
    second: make
    write(first, second.left)
    first.release
    second.release
    return
"#,
        &["type-invalid-derived-target-group-argument"],
    );
}

#[test]
fn derived_group_rejects_a_sibling_slot_from_the_same_arena() {
    expect_flaws_with(
        ARENA,
        r#"
write(arena ref{storage} Arena, slot mut{storage.left} Slot):
    slot.value:: replace(slot.value, 23)
    return
run:
    arena: make
    write(arena, arena.right)
    arena.release
    return
"#,
        &["type-invalid-derived-target-group-argument"],
    );
}

#[test]
fn returning_a_block_cannot_hide_local_arena_storage() {
    expect_flaws_with(
        ARENA,
        r#"
escape Block:
    arena: make
    return reserve_left(arena)
"#,
        &[
            "type-reference-return-local-target",
            "type-terminal-value-not-consumed",
        ],
    );
}
