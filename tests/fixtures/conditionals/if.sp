* ngspice conditionals (ngspice manual §2.15 examples, extended), hand-written
* for tests/test_conditionals.rs
.param ok=0 ok2=1 m0=0 m1=1
.subckt sel a b k=0
.if (k == 0)
R1 a b 1k
.elseif (k == 1)
.if (ok2)
R1 a b 2k
.else
R1 a b 3k
.endif
.else
R1 a b 4k
.endif
.ends sel
V1 1 0 1
.if (ok && ok2)
R11 1 0 2
.else
R11 1 0 0.5
.endif
X0 1 0 sel k=0
X1 1 0 sel k=1
X2 1 0 sel k=2
.if (m0 == 1)
.model D1 D is=1e-14
.elseif (m1 == 1)
.model D1 D is=2e-14
.else
.model D1 D is=3e-14
.endif
D1 1 2 D1
R2 2 0 1k
.op
.end
