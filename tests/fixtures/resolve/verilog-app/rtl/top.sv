`include "defs.svh"
module top (input logic clk, output logic [7:0] q);
  counter #(.W(8)) u_counter (.clk(clk), .q(q));
endmodule
