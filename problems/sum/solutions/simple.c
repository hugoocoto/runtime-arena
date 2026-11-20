/* Reads everything into an array with scanf: correct, slower and heavier. */
#include <stdio.h>
#include <stdlib.h>
int main(void) {
    long long n;
    if (scanf("%lld", &n) != 1) return 1;
    long long *a = malloc(sizeof *a * (size_t)n), s = 0;
    for (long long i = 0; i < n; i++) if (scanf("%lld", &a[i]) != 1) return 1;
    for (long long i = 0; i < n; i++) s += a[i];
    printf("%lld\n", s);
    return 0;
}
