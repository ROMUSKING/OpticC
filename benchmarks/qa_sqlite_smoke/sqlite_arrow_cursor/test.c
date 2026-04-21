
#include <stdio.h>

struct BtCursor {
    int state;
    int step;
};

int main() {
    struct BtCursor cur;
    struct BtCursor *pCur = &cur;
    pCur->state = 4;
    pCur->step = 1;

    while (pCur->step < 6) {
        pCur->state = pCur->state + pCur->step;
        pCur->step++;
    }

    printf("%d\n", pCur->state + pCur->step);
    return 0;
}
