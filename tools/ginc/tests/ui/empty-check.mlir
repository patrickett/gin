module {
  func.func @five() -> i64 {
    %const0 = arith.constant 5 : i64
    return %const0 : i64
  }
}
