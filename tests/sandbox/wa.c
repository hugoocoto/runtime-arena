/* Correct except when the first number is 10, where it is off by one. */
#include <stdio.h>
int main(void) { long a, b; if (scanf("%ld %ld", &a, &b) != 2) return 2; printf("%ld\n", a + b + (a == 10)); return 0; }
