`include "defs.svh"
package store_pkg; typedef int row_t; endpackage
module store #(parameter W = 8) (input logic clk, output logic [W-1:0] q);
  import store_pkg::*;
  counter #(.W(W)) u_counter (.clk(clk), .q(q));
  function automatic int add(int a); return a + 1; endfunction
  task reset(); endtask
  always_ff @(posedge clk) q <= add(q);
endmodule
