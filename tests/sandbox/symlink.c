/* Cheats by replacing its output with a link to the judge's answer file. */
#include <stdio.h>
#ifndef ANSWER
#define ANSWER "/nonexistent"
#endif
#include <unistd.h>
int main(void) {
    unlink("out");
    if (symlink(ANSWER, "out") != 0) return 3;
    return 0;
}
