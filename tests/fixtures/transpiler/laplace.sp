laplace sources: a lead-lag V output and a two-pole I output (ngspice input)
V1 in 0 DC 0 AC 1
B1 o1 0 V=Laplace(V(in), (1+s*1e-3)/(1+s*1e-6))
B2 o2 0 I=Laplace(V(in,o1), 2/(1+s*2e-4+s^2*1e-8))
R1 o2 0 1k
.ac dec 10 1 1meg
.end
