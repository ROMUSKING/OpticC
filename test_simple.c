struct U { int *p; int *q; };
struct S { int x; struct U u; };
struct S s = { 1, { 0 } };
