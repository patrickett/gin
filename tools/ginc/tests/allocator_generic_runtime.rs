mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn imported_default_stores_specialize_construction_and_copying() {
    let package = TempPackage::new("generic_stores");
    package.write_flask("generic_stores");
    package.write(
        "store/store.gin",
        r#"Word is in 0 to 255
Result is Word or Failed
Word has
    finish(self):
        return
    copy(self ref) Self: return deref self
#phantom(unit)
Count(unit) is in 0 to 255
Block(element) has value element, capacity Count(element)
take(element, b Block(element)) element:
    Block(value, capacity) := b
    return value
Block(element) has
    finish(self):
        return
Count(unit) has
    finish(self):
        return
Result has
    finish(self):
        return
Error is Empty or Full
Attempt(element) is Block(element) or Failed(Error)
Error has
    finish(self):
        return
Attempt(element) has
    finish(self):
        return
Storage has
    reclaim(self mut, b Block(element))
    release(self)
reclaim(b Block(element), a mut has Storage): return a.reclaim(b)
GlobalStore has Storage
    reserve(self mut, value own element, c Count(element)) Attempt(element):
        block Block(element): Block(value, capacity: c)
        return block
    copy(self ref) Self: return Self()
    reclaim(self mut, b Block(element)):
        return
    release(self):
        return
"#,
    );
    package.write(
        "holder/holder.gin",
        r#"use generic_stores.store GlobalStore, Block, Attempt, Count, Word, Error, take, reclaim
Holder(element, store: GlobalStore) has
    value element
    allocator store
    empty(element) Holder(element): return Holder.new(element, 0 as element)
    new(element, value own element) Holder(element): return Holder.new(element, value, GlobalStore())
    new(element, value own element, a own store) Self:
        Holder.require(element, value)
        attempt: a.reserve(value, 1 as Count(element))
        block: Holder.unwrap(element, attempt)
        stored: take(element, block)
        return Self(value: stored, allocator: a)
    require(element, value ref element):
        return
    unwrap(element, a Attempt(element)) Block(element):
        return when a is
            Block(block) then block
            Failed(error) then Holder.failed(element, error)
    failed(element, error Error) Block(element): return Holder.failed(element, error)
    copy(self ref) Self: return self.copy_into(store, self.allocator.copy)
    copy_into(self ref, destination, a own destination) Holder(element, destination):
        return Holder.new(element, element.copy(self.value), a)
    release(self):
        Holder(value, allocator: a) := self
        block Block(element): Block(value, capacity: 1 as Count(element))
        reclaim(block, a)
        a.release
        return
"#,
    );
    let input = package.write(
        "main.gin",
        r#"use generic_stores.holder Holder
use generic_stores.store Word
#entry
main:
    empty: Holder.empty(Word)
    empty_copy: empty.copy
    empty.release
    empty_copy.release
    original: Holder.new(Word, 7 as Word)
    copied: original.copy
    first is in Word: original.value
    second is in Word: copied.value
    original.release
    copied.release
    return when first = 7 and second = 7 then 0 else 1
"#,
    );
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("program-{profile:?}"));
        let run = compile_and_run_exe(input.clone(), &executable, profile, "", "run stores");
        assert!(run.status.success(), "{profile:?}");
    }
}
