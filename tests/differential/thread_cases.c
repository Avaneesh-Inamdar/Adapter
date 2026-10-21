/* Same-artifact threading probes: deterministic JSONL, no addresses or clock.
 * Every family synchronizes before reading shared state, so a correct runtime
 * answers identically on Darwin and through Adapter regardless of scheduling.
 */
extern int printf(const char *, ...);

typedef struct { long sig; char opaque[56]; } mutex_t;
typedef struct { long sig; char opaque[40]; } cond_t;
typedef struct { long sig; char opaque[56]; } attr_t;
typedef struct { long sig; char opaque[8]; } once_t;

extern int pthread_create(void **, const void *, void *(*)(void *), void *);
extern int pthread_join(void *, void **);
extern void *pthread_self(void);
extern int pthread_equal(void *, void *);
extern int pthread_once(once_t *, void (*)(void));
extern int pthread_key_create(unsigned *, void (*)(void *));
extern void *pthread_getspecific(unsigned);
extern int pthread_setspecific(unsigned, const void *);
extern int pthread_mutex_init(mutex_t *, const void *);
extern int pthread_mutex_lock(mutex_t *);
extern int pthread_mutex_unlock(mutex_t *);
extern int pthread_cond_init(cond_t *, const void *);
extern int pthread_cond_wait(cond_t *, mutex_t *);
extern int pthread_cond_signal(cond_t *);
extern int pthread_attr_init(attr_t *);
extern int pthread_attr_destroy(attr_t *);
extern int pthread_attr_setstacksize(attr_t *, unsigned long);

static const char *families[8] = {"createjoin", "identity", "mutex", "once",
                                  "tls", "cond", "attr", "retval"};

static mutex_t counter_lock;
static long counter;
static long once_count;
static long once_rc[2];
/* Darwin's PTHREAD_ONCE_INIT is __sig == 0x30B1BCBA, not zero. */
static once_t once_control = {0x30b1bcba, {0}};
static unsigned tls_key;
static long tls_ok;
static mutex_t gate_lock;
static cond_t gate_cond;
static long gate_open;

static void *seed_worker(void *arg) {
    return (void *)((long)arg * 7 + 1);
}
static void *identity_worker(void *arg) {
    long code = pthread_equal(pthread_self(), arg) ? 1 : 0;
    code = code * 10 + (pthread_equal(pthread_self(), pthread_self()) ? 1 : 0);
    return (void *)code;
}
static void *counter_worker(void *arg) {
    long rounds = (long)arg;
    for (long i = 0; i < rounds; i++) {
        pthread_mutex_lock(&counter_lock);
        counter++;
        pthread_mutex_unlock(&counter_lock);
    }
    return 0;
}
static void once_body(void) {
    once_count++;
}
static void *once_worker(void *arg) {
    long slot = (long)arg;
    int rc = pthread_once(&once_control, once_body);
    pthread_mutex_lock(&counter_lock);
    once_rc[slot] = rc;
    pthread_mutex_unlock(&counter_lock);
    return 0;
}
static void *tls_worker(void *arg) {
    long value = (long)arg * 11;
    if (pthread_setspecific(tls_key, (const void *)value) != 0)
        return 0;
    if (pthread_getspecific(tls_key) == (void *)value) {
        pthread_mutex_lock(&counter_lock);
        tls_ok++;
        pthread_mutex_unlock(&counter_lock);
    }
    return 0;
}
static void *gate_worker(void *arg) {
    (void)arg;
    pthread_mutex_lock(&gate_lock);
    gate_open = 1;
    pthread_cond_signal(&gate_cond);
    pthread_mutex_unlock(&gate_lock);
    return 0;
}
static void *attr_worker(void *arg) {
    (void)arg;
    return (void *)1;
}
static void *retval_worker(void *arg) {
    return (void *)((long)arg * 13 + 5);
}

int main(void) {
    for (int index = 0; index < DIFF_CASE_COUNT; index++) {
        long result[3];
        int used = 1;
        switch (index % 8) {
        case 0: {
            void *a = 0, *b = 0, *ra = 0, *rb = 0;
            int ca = pthread_create(&a, 0, seed_worker, (void *)2);
            int cb = pthread_create(&b, 0, seed_worker, (void *)3);
            int ja = pthread_join(a, &ra);
            int jb = pthread_join(b, &rb);
            result[0] = ca + cb;
            result[1] = ja + jb;
            result[2] = (long)ra + (long)rb;
            used = 3;
            break;
        }
        case 1: {
            void *worker = 0, *value = 0;
            pthread_create(&worker, 0, identity_worker, pthread_self());
            pthread_join(worker, &value);
            result[0] = pthread_self() != 0;
            result[1] = pthread_equal(pthread_self(), pthread_self());
            result[2] = (long)value;
            used = 3;
            break;
        }
        case 2: {
            void *a = 0, *b = 0;
            pthread_mutex_init(&counter_lock, 0);
            counter = 0;
            pthread_create(&a, 0, counter_worker, (void *)50);
            pthread_create(&b, 0, counter_worker, (void *)50);
            pthread_join(a, 0);
            pthread_join(b, 0);
            result[0] = counter;
            result[1] = 1;
            used = 2;
            break;
        }
        case 3: {
            void *a = 0, *b = 0;
            once_count = 0;
            pthread_mutex_init(&counter_lock, 0);
            once_rc[0] = once_rc[1] = -1;
            pthread_create(&a, 0, once_worker, (void *)0);
            pthread_create(&b, 0, once_worker, (void *)1);
            pthread_join(a, 0);
            pthread_join(b, 0);
            result[0] = once_count;
            result[1] = once_rc[0] + once_rc[1];
            used = 2;
            break;
        }
        case 4: {
            void *a = 0, *b = 0;
            tls_ok = 0;
            tls_key = 0;
            pthread_key_create(&tls_key, 0);
            pthread_create(&a, 0, tls_worker, (void *)4);
            pthread_create(&b, 0, tls_worker, (void *)9);
            pthread_join(a, 0);
            pthread_join(b, 0);
            result[0] = tls_ok;
            result[1] = pthread_getspecific(tls_key) == 0;
            used = 2;
            break;
        }
        case 5: {
            void *worker = 0;
            pthread_mutex_init(&gate_lock, 0);
            pthread_cond_init(&gate_cond, 0);
            gate_open = 0;
            pthread_mutex_lock(&gate_lock);
            pthread_create(&worker, 0, gate_worker, 0);
            while (!gate_open)
                pthread_cond_wait(&gate_cond, &gate_lock);
            int unlock = pthread_mutex_unlock(&gate_lock);
            pthread_join(worker, 0);
            result[0] = gate_open;
            result[1] = unlock;
            used = 2;
            break;
        }
        case 6: {
            attr_t attributes;
            void *worker = 0, *value = 0;
            int init = pthread_attr_init(&attributes);
            int size = pthread_attr_setstacksize(&attributes, 1 << 20);
            int create = pthread_create(&worker, &attributes, attr_worker, 0);
            int join = pthread_join(worker, &value);
            pthread_attr_destroy(&attributes);
            result[0] = init + size + create + join;
            result[1] = (long)value;
            used = 2;
            break;
        }
        case 7: {
            void *a = 0, *b = 0, *ra = 0, *rb = 0;
            pthread_create(&a, 0, retval_worker, (void *)1);
            pthread_create(&b, 0, retval_worker, (void *)2);
            pthread_join(a, &ra);
            pthread_join(b, &rb);
            result[0] = (long)ra == 18;
            result[1] = (long)rb == 31;
            used = 2;
            break;
        }
        }
        printf("{\"family\":\"%s\",\"id\":\"%s:%d\",\"index\":%d,\"result\":[",
               families[index % 8], families[index % 8], index, index);
        for (int i = 0; i < used; i++)
            printf(i ? ",%ld" : "%ld", result[i]);
        printf("]}\n");
    }
    return 0;
}
