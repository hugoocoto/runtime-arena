/* Forks until refused. Prints the sum only if the process limit stops it. */
#include <errno.h>
#include <stdio.h>
#include <unistd.h>

int main(void) {
    long a, b;
    if (scanf("%ld %ld", &a, &b) != 2) return 2;
    for (int i = 0; i < 200; i++) {
        pid_t pid = fork();
        if (pid == 0) {
            pause();
            _exit(0);
        }
        if (pid < 0) {
            if (errno != EAGAIN) return 3;
            printf("%ld\n", a + b);
            return 0;
        }
    }
    return 4;
}
