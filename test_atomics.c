int counter = 0;

int test_atomic_add() {
    return __sync_fetch_and_add(&counter, 5);
}

int test_atomic_cas() {
    int expected = 5;
    return __sync_val_compare_and_swap(&counter, expected, 10);
}

_Thread_local int tls_var = 42;

int main() {
    test_atomic_add();
    test_atomic_cas();
    return tls_var;
}
