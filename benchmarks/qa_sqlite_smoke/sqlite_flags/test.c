
#include <stdio.h>

typedef unsigned char u8;
typedef unsigned int u32;

#define MEM_Int 0x0001
#define MEM_Str 0x0002
#define MEM_Null 0x0004
#define ALWAYS(X) __builtin_expect(!!(X), 1)
#define FLAG_SET(V, F) (((V) & (F)) != 0)

static int classify_flags(u32 flags) {
    if (ALWAYS(FLAG_SET(flags, MEM_Int))) {
        return 3;
    }
    if (FLAG_SET(flags, MEM_Str)) {
        return 2;
    }
    if (FLAG_SET(flags, MEM_Null)) {
        return 1;
    }
    return 0;
}

int main() {
    u32 flags[4] = { MEM_Int, MEM_Str, MEM_Int | MEM_Str, MEM_Null };
    int i = 0;
    int score = 0;
    while (i < 4) {
        score += classify_flags(flags[i]);
        i = i + 1;
    }
    printf("%d\n", score);
    return 0;
}
