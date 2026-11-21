#include <pthread.h>
#include <stdio.h>

static long a, b;
static long part[4];

static void *add(void *arg) {
    long i = (long)arg;
    part[i] = i == 0 ? a : i == 1 ? b : 0;
    return NULL;
}

int main(void) {
    if (scanf("%ld %ld", &a, &b) != 2) return 2;
    pthread_t threads[4];
    for (long i = 0; i < 4; i++) {
        if (pthread_create(&threads[i], NULL, add, (void *)i) != 0) return 3;
    }
    long sum = 0;
    for (int i = 0; i < 4; i++) {
        pthread_join(threads[i], NULL);
        sum += part[i];
    }
    printf("%ld\n", sum);
    return 0;
}
