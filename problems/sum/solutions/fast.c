/* Streams the input with a hand-written reader: fast and constant memory. */
#include <stdio.h>
#include <unistd.h>
static char buf[1 << 16];
static size_t len, pos;
static int next(void) {
    if (pos == len) { len = (size_t)read(0, buf, sizeof buf); pos = 0; if (len <= 0) return -1; }
    return buf[pos++];
}
static long long read_int(void) {
    int c = next();
    while (c == ' ' || c == '\n') c = next();
    int neg = c == '-';
    if (neg) c = next();
    long long v = 0;
    for (; c >= '0' && c <= '9'; c = next()) v = v * 10 + (c - '0');
    return neg ? -v : v;
}
int main(void) {
    long long n = read_int(), s = 0;
    for (long long i = 0; i < n; i++) s += read_int();
    printf("%lld\n", s);
    return 0;
}
