#include <omp.h>
#include <stdio.h>

int main(void) {
    long a, b;
    if (scanf("%ld %ld", &a, &b) != 2) return 2;
    long values[] = {a, b, 0, 0};
    long sum = 0;
#pragma omp parallel for reduction(+ : sum)
    for (int i = 0; i < 4; i++) sum += values[i];
    printf("%ld\n", sum);
    return 0;
}
