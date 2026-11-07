module {
  func.func @Level.add(%arg0: i64, %arg1: i64) -> i64 {
    %t4 = arith.addi %arg0, %arg1 : i64
    return %t4 : i64
  }
  func.func @level() {
    %const0 = arith.constant 7 : i64
    %const1 = arith.constant 5 : i64
    %call4 = func.call @Level.add(%const0, %const1) : (i64, i64) -> i64
    %w5_lo = arith.constant 0 : i64
    %w5_wd = arith.constant 10 : i64
    %w5_d = arith.subi %call4, %w5_lo : i64
    %w5_r = arith.remsi %w5_d, %w5_wd : i64
    %w5_p = arith.addi %w5_r, %w5_wd : i64
    %w5_m = arith.remsi %w5_p, %w5_wd : i64
    %w5_out = arith.addi %w5_m, %w5_lo : i64
    return
  }
}
