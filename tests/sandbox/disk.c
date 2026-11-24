/* Fills the box with 1 MiB files. Prints the sum only if the disk limit stops it
 * (the judge tests run with a 16 MiB box). */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#define MIB (1 << 20)

static char block[MIB];

int main(void) {
    long a, b;
    char name[32];
    int files = 0;
    if (scanf("%ld %ld", &a, &b) != 2) return 2;
    memset(block, 1, MIB);
    for (;;) {
        if (files > 32) return 4;
        snprintf(name, sizeof name, "f%d", files);
        int fd = open(name, O_CREAT | O_WRONLY, 0600);
        if (fd < 0) {
            if (errno == ENOSPC) break;
            return 3;
        }
        files++;
        ssize_t written = write(fd, block, MIB);
        int error = errno;
        close(fd);
        if (written == MIB) continue;
        if (written >= 0 || error == ENOSPC) break;
        return 3;
    }
    /* Free the space so the answer can be written. */
    for (int i = 0; i < files; i++) {
        snprintf(name, sizeof name, "f%d", i);
        unlink(name);
    }
    printf("%ld\n", a + b);
    return 0;
}
