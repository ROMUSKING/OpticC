#define SQUARE(x) ((x) * (x))
#define MAX(a, b) ((a) > (b) ? (a) : (b))

int main() {
    int x = SQUARE(5);
    int y = MAX(3, 4);
    return x + y;
}
