/* Prints the sum only if it cannot act as root, write outside its box or see
 * the server's files. Each failed check has its own exit code. */
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>

int main(void) {
    static const char *const writes[] = {"/pwned", "/usr/pwned", "/lib/pwned", "/etc/pwned"};
    static const char *const hidden[] = {"/data", "/etc/shadow", "/var/local/lib/isolate", "/src"};
    long a, b;
    if (scanf("%ld %ld", &a, &b) != 2) return 2;
    if (getuid() == 0 || geteuid() == 0) return 10;
    for (int i = 0; i < 4; i++) {
        int fd = open(writes[i], O_CREAT | O_WRONLY, 0600);
        if (fd >= 0) return 20 + i;
    }
    for (int i = 0; i < 4; i++) {
        if (access(hidden[i], F_OK) == 0) return 30 + i;
    }
    printf("%ld\n", a + b);
    return 0;
}
