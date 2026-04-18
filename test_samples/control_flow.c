/* Tests for/while/if to validate pipeline past simple.c */
int fib(int n) {
    if (n <= 1) return n;
    int a = 0, b = 1, c;
    int i = 2;
    while (i <= n) {
        c = a + b;
        a = b;
        b = c;
        i = i + 1;
    }
    return b;
}

int main() {
    int x = fib(10);
    return x;
}
