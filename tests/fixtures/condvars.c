typedef unsigned long pthread_t;
typedef struct {
    long sig;
    char opaque[56];
} pthread_mutex_t;
typedef struct {
    long sig;
    char opaque[40];
} pthread_cond_t;
typedef struct {
    long sig;
    char opaque[8];
} pthread_condattr_t;
typedef struct {
    long sec, nsec;
} Timespec;
typedef struct {
    long sec;
    int usec;
} Timeval;
extern int pthread_mutex_init(pthread_mutex_t *, const void *);
extern int pthread_mutex_lock(pthread_mutex_t *);
extern int pthread_mutex_trylock(pthread_mutex_t *);
extern int pthread_mutex_unlock(pthread_mutex_t *);
extern int pthread_mutex_destroy(pthread_mutex_t *);
extern int pthread_cond_init(pthread_cond_t *, const pthread_condattr_t *);
extern int pthread_cond_destroy(pthread_cond_t *);
extern int pthread_cond_wait(pthread_cond_t *, pthread_mutex_t *);
extern int pthread_cond_timedwait(pthread_cond_t *, pthread_mutex_t *, const Timespec *);
extern int pthread_cond_signal(pthread_cond_t *);
extern int pthread_cond_broadcast(pthread_cond_t *);
extern int pthread_condattr_init(pthread_condattr_t *);
extern int pthread_condattr_destroy(pthread_condattr_t *);
extern int pthread_condattr_getpshared(const pthread_condattr_t *, int *);
extern int pthread_condattr_setpshared(pthread_condattr_t *, int);
extern int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
extern int pthread_join(pthread_t, void **);
extern int gettimeofday(Timeval *, void *);
extern int puts(const char *);
static pthread_mutex_t mutex = {0x32AAABA7, {0}};
static pthread_cond_t cond = {0x3CB0B1BB, {0}};
static pthread_cond_t handoff = {0x3CB0B1BB, {0}};
static int ready, go, done, signaled_ready, signaled_go;
static void *broadcast_worker(void *arg) {
    int id = (int)(unsigned long)arg;
    if (pthread_mutex_lock(&mutex))
        return (void *)1;
    ready++;
    if (pthread_cond_broadcast(&cond))
        return (void *)2;
    while (!go)
        if (pthread_cond_wait(&cond, &mutex))
            return (void *)3;
    done++;
    if (pthread_cond_broadcast(&cond))
        return (void *)4;
    if (pthread_mutex_unlock(&mutex))
        return (void *)5;
    return (void *)(unsigned long)(100 + id);
}
static void *signal_worker(void *arg) {
    (void)arg;
    if (pthread_mutex_lock(&mutex))
        return (void *)1;
    signaled_ready = 1;
    if (pthread_cond_signal(&handoff))
        return (void *)2;
    while (!signaled_go)
        if (pthread_cond_wait(&handoff, &mutex))
            return (void *)3;
    if (pthread_mutex_unlock(&mutex))
        return (void *)4;
    return (void *)7;
}
int main(void) {
    pthread_condattr_t attr;
    int shared = -1;
    if (pthread_condattr_init(&attr))
        return 1;
    if (pthread_condattr_getpshared(&attr, &shared) || shared != 2)
        return 2;
    if (pthread_condattr_setpshared(&attr, 1) || pthread_condattr_getpshared(&attr, &shared) ||
        shared != 1)
        return 3;
    if (pthread_condattr_setpshared(&attr, 3) != 22)
        return 4;
    if (pthread_cond_init((pthread_cond_t *)0, &attr) != 22)
        return 5;
    if (pthread_condattr_destroy(&attr) || pthread_condattr_destroy(&attr) != 22)
        return 6;
    pthread_cond_t bad = {0, {0}};
    if (pthread_cond_signal(&bad) != 22 || pthread_cond_wait(&bad, &mutex) != 22)
        return 7;
    pthread_cond_t initialized = {0, {0}};
    if (pthread_cond_init(&initialized, (pthread_condattr_t *)0))
        return 8;
    if (pthread_cond_signal(&initialized))
        return 9;
    if (pthread_cond_destroy(&initialized) || pthread_cond_destroy(&initialized) != 22)
        return 10;
    pthread_t threads[4];
    for (int i = 0; i < 4; i++)
        if (pthread_create(&threads[i], (void *)0, broadcast_worker, (void *)(unsigned long)i))
            return 11;
    if (pthread_mutex_lock(&mutex))
        return 12;
    while (ready < 4)
        if (pthread_cond_wait(&cond, &mutex))
            return 13;
    go = 1;
    if (pthread_cond_broadcast(&cond))
        return 14;
    while (done < 4)
        if (pthread_cond_wait(&cond, &mutex))
            return 15;
    if (pthread_mutex_unlock(&mutex))
        return 16;
    for (int i = 0; i < 4; i++) {
        void *result = (void *)0;
        if (pthread_join(threads[i], &result) || result != (void *)(unsigned long)(100 + i))
            return 17;
    }
    pthread_t one;
    if (pthread_create(&one, (void *)0, signal_worker, (void *)0))
        return 18;
    if (pthread_mutex_lock(&mutex))
        return 19;
    while (!signaled_ready)
        if (pthread_cond_wait(&handoff, &mutex))
            return 20;
    signaled_go = 1;
    if (pthread_cond_signal(&handoff))
        return 21;
    if (pthread_mutex_unlock(&mutex))
        return 22;
    void *result = (void *)0;
    if (pthread_join(one, &result) || result != (void *)7)
        return 23;
    Timeval now;
    if (gettimeofday(&now, (void *)0))
        return 24;
    Timespec deadline = {now.sec, now.usec * 1000 + 50000000};
    if (deadline.nsec >= 1000000000) {
        deadline.sec++;
        deadline.nsec -= 1000000000;
    }
    pthread_cond_t timed = {0, {0}};
    if (pthread_cond_init(&timed, (pthread_condattr_t *)0))
        return 25;
    if (pthread_mutex_lock(&mutex))
        return 26;
    if (pthread_cond_timedwait(&timed, &mutex, &deadline) != 60)
        return 27;
    if (pthread_mutex_trylock(&mutex) != 16)
        return 28;
    if (pthread_mutex_unlock(&mutex))
        return 29;
    Timespec past = {0, 0};
    if (pthread_mutex_lock(&mutex))
        return 30;
    if (pthread_cond_timedwait(&timed, &mutex, &past) != 60)
        return 31;
    if (pthread_mutex_unlock(&mutex))
        return 32;
    Timespec invalid = {now.sec, 1000000000};
    if (pthread_mutex_lock(&mutex))
        return 33;
    if (pthread_cond_timedwait(&timed, &mutex, &invalid) != 22)
        return 34;
    if (pthread_mutex_unlock(&mutex))
        return 35;
    if (pthread_cond_destroy(&timed))
        return 36;
    if (pthread_cond_destroy(&cond) || pthread_cond_destroy(&handoff))
        return 37;
    if (pthread_mutex_destroy(&mutex))
        return 38;
    puts("conds:wait, signal, broadcast and timedwait passed");
    return 0;
}
