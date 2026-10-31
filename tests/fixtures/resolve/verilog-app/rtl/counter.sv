module counter #(parameter W = 8) (input logic clk, output logic [W-1:0] q);
  function automatic logic [W-1:0] next(logic [W-1:0] v); return v + 1; endfunction
  always_ff @(posedge clk) q <= next(q);
endmodule
