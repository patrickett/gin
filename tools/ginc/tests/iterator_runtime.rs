mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn library_iterator_advances_and_stays_exhausted() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("iterator_runtime");
        package.write_flask("iterator_runtime");
        package.write(
            "iter/iter.gin",
            include_str!("../../../modules/core/iter/iter.gin"),
        );
        let input = package.write(
            "main.gin",
            r#"use iterator_runtime.iter Iterator, Step

Byte is in 0 to 255
Byte has
    replace(previous Self, next Self) Self: return next
    finish(self):
        return

Countdown has Iterator(Byte)
    remaining Byte

    next(self mut) Step(Byte):
        value is in Byte: self.remaining
        when value = 0 then
            return Done
        self.remaining:: Byte.replace(self.remaining, (value - 1) as Byte)
        return Yield(value as Byte)

    finish(self):
        return

equal(actual is in Byte, expected is in Byte) Byte:
    return when actual = expected then 0 else 1

expect(s Step(Byte), expected Byte) Byte:
    return when s is
        Yield(value) then equal(value, expected)
        Done then 2

exhausted(s Step(Byte)) Byte:
    return when s is
        Yield(value) then 1
        Done then 0

#entry
main:
    counter: Countdown(remaining: 3)
    first is in Byte: expect(counter.next, 3)
    second is in Byte: expect(counter.next, 2)
    third is in Byte: expect(counter.next, 1)
    done is in Byte: exhausted(counter.next)
    again is in Byte: exhausted(counter.next)
    counter.finish
    empty: Countdown(remaining: 0)
    initial is in Byte: exhausted(empty.next)
    empty.finish
    return first + second + third + done + again + initial
"#,
        );
        let executable = package.join("program");
        let output = compile_and_run_exe(input, &executable, profile, "", "run iterator");
        assert_eq!(output.status.code(), Some(0), "{profile:?}");
    }
}
