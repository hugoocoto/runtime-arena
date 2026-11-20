/* Touches 512 MiB page by page; volatile so the compiler cannot elide it. */
#include <stdio.h>
#include <stdlib.h>
int main(void) {
    size_t n = (size_t)512 << 20;
    volatile char *p = malloc(n);
    if (!p) return 3;
    for (size_t i = 0; i < n; i += 4096) p[i] = (char)i;
    printf("%d\n", p[n / 2]);
    return 0;
}
