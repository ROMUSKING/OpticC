
#include <stdio.h>

struct ParseState {
    int opcode;
    int flags;
    int steps;
    int acc;
};

#define OPFLAG_IN1 0x01
#define OP_Column 2

int main() {
    struct ParseState st;
    st.opcode = OP_Column;
    st.flags = OPFLAG_IN1;
    st.steps = 0;
    st.acc = 0;

    while (st.steps < 8) {
        if ((st.flags & OPFLAG_IN1) != 0) {
            st.acc = st.acc + st.opcode;
        } else {
            st.acc = st.acc - 1;
        }
        st.steps = st.steps + 1;
    }

    printf("%d\n", st.acc + st.steps);
    return 0;
}
