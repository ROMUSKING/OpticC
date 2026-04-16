/* OPTIC RECONSTRUCTED FILE */
/* Taint Tracking Shadow Comments */

void copy_string(char *dest, const char *src) {
    // [OPTIC ERROR] strcpy(dest, src); - potential buffer overflow
    strcpy(dest, src);
}

void format_output(char *buf, const char *user_input) {
    // [OPTIC ERROR] sprintf(buf, user_input); - potential buffer overflow
    sprintf(buf, user_input);
}

char *create_buffer(size_t size) {
    // [OPTIC ERROR] malloc(size); - unchecked allocation
    char *buf = malloc(size);
    return buf;
}

void process_data(char *data) {
    // [OPTIC ERROR] free(data); - memory freed, potential use-after-free
    free(data);
}
