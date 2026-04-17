void null_deref() {
    char *p = NULL;
    *p = 'x';
}

void use_after_free() {
    char *p = malloc(10);
    free(p);
    p[0] = 'x';
}
