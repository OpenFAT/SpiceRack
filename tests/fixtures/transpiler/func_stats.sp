func_stats: functions whose bodies draw
* Made up (tests/fixtures/transpiler/README.md). For Spectre, each call
* of a .func whose body calls a statistical function is inlined, `(body)`
* with each parameter `(arg)`, and each inlined call is drawn on its own:
* a Spectre user function's body may reference only its arguments and
* temp/tnom (Spectre Reference 19.1 p.481), so a draw cannot sit in it,
* and ngspice draws a .func body's call once per call of the function.
* The function is not written. Oracle: the old writer's output, byte for
* byte (the extended baseline).
* Probes: a call at the top (p), two in one value (R1, the second
* spelled F), a call nested in its own argument and one whose argument
* calls a function (R2), an argument that draws, used twice (R3), a call
* in a subcircuit (R4: mismatch), a function nothing calls (dropped).
.func f(x) {x*agauss(1, 0.1, 3)}
.func g(a, b) 'a + aunif(b, 0.1)'
.func h(x) {x*2}
.func d(x) {x + x*agauss(0, 1, 1)}
.func unused(x) {agauss(x, 1, 1)}
.param p=f(2)
V1 a 0 1
R1 a b {f(1k) + F( 2k )}
R2 b 0 {g(1, h(3)) + f(f(1))}
R3 b c {d(aunif(1k, 10))}
.subckt s n params: w=1k
R4 n 0 {f(w) + 1}
.ends s
X1 c s
.op
.end
