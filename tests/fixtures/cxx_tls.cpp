typedef unsigned long pthread_t;
extern "C" int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
extern "C" int pthread_join(pthread_t, void **);
extern "C" int puts(const char *);
extern "C" int write(int, const void *, unsigned long);
struct Tracked {
    int value;
    explicit Tracked(int v) : value(v) {}
    ~Tracked();
};
thread_local Tracked tracked(7);
Tracked::~Tracked() { write(1, "T", 1); }
static int touch(char marker) {
    if (tracked.value != 7)
        return 0;
    write(1, &marker, 1);
    return 1;
}
static void *worker(void *arg) {
    if (!touch('C'))
        return (void *)1;
    return arg;
}
int main() {
    pthread_t threads[3];
    for (int i = 0; i < 3; i++)
        if (pthread_create(&threads[i], (void *)0, worker, (void *)(unsigned long)(i + 1)))
            return 1;
    for (int i = 0; i < 3; i++) {
        void *result = (void *)0;
        if (pthread_join(threads[i], &result) || result != (void *)(unsigned long)(i + 1))
            return 2;
    }
    if (!touch('M'))
        return 3;
    puts("cxx:done");
    return 0;
}
