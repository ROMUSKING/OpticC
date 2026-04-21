
#include <stdio.h>

typedef unsigned char u8;
typedef unsigned int u32;

static u32 decode_varint32(const u8 *data) {
    u32 value = 0;
    int i = 0;
    int keep_reading = 1;
    while (i < 4) {
        if (keep_reading) {
            value = (value << 7) | (u32)(data[i] & 0x7f);
            if ((data[i] & 0x80) == 0) {
                keep_reading = 0;
            }
        }
        i = i + 1;
    }
    return value;
}

int main() {
    u8 bytes[4] = { 0x81, 0x82, 0x03, 0x00 };
    printf("%u\n", decode_varint32(bytes));
    return 0;
}
