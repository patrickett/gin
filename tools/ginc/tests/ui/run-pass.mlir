// gin module: ui_fixture
module {
  func.func @"ui_fixture.start"() -> i64 {
    %const0 = arith.constant 5 : i64
    return %const0 : i64
  }
  func.func @main() -> i32 {
    %value = func.call @"ui_fixture.start"() : () -> i64
    %status = arith.trunci %value : i64 to i32
    return %status : i32
  }
}
