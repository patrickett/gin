mod common;
use common::run_in_profiles;
use test_fixtures::TempPackage;

#[test]
fn caller_owned_arena_preserves_blocks_across_reservation_and_bookkeeping() {
    let package = TempPackage::new("arena_borrowing");
    package.write_flask("arena_borrowing");
    package.write(
        "arena/arena.gin",
        &format!("{}replace(previous Byte, next Byte) Byte: return next\nread(value ref Byte) Byte: return deref value\nSlot has\n    value Byte\n    reclaim(self):\n        return\nreplace_slot(previous Slot, next Slot) Slot:\n    previous.reclaim\n    return next\nBlock has\n    value ref Byte\n    finish(self):\n        return\nArena has\n    left Slot\n    right Slot\n    used Byte\n    reserve_left(self mut) Block:\n        self.used:: replace(self.used, 1)\n        return Block(value: self.left.value)\n    reserve_right(self mut) Block:\n        self.used:: replace(self.used, 2)\n        return Block(value: self.right.value)\n    reset_right(self mut):\n        self.right:: replace_slot(self.right, Slot(value: 0))\n        return\n    release(self):\n        Arena(left, right, used) := self\n        left.reclaim\n        right.reclaim\n        return\nmake Arena: Arena(left: Slot(value: 7), right: Slot(value: 19), used: 0)\n", test_fixtures::preludes::BYTE_PRELUDE),
    );
    package.write(
        "lease/lease.gin",
        r#"use arena_borrowing.arena Arena, Block
Lease has
    block Block
    finish(self):
        Lease(block) := self
        block.finish
        return
reserve(arena mut Arena) Lease: return Lease(block: arena.reserve_left)
move_lease(lease own Lease) Lease: return lease
"#,
    );
    let input = package.write(
        "main.gin",
        r#"use arena_borrowing.arena Byte, make, read
use arena_borrowing.lease reserve, move_lease
#entry
main:
    arena: make
    lease: move_lease(reserve(arena))
    second: arena.reserve_right
    second_value is in Byte: read(second.value)
    used is in Byte: arena.used
    second.finish
    arena.reset_right
    first_value is in Byte: read(lease.block.value)
    lease.finish
    arena.release
    return when first_value = 7 and second_value = 19 and used = 2 then 0 else 1
"#,
    );
    run_in_profiles(&package, input, "run borrowing");
}
