/* pthread QoS consumer: classes and relative priorities are recorded per thread,
   attributes carry a requested class, invalid arguments are rejected. */
typedef unsigned long pthread_t;
typedef struct { long sig; char opaque[56]; } pthread_attr_t;

extern int puts(const char *);
extern int pthread_attr_init(pthread_attr_t *attr);
extern int pthread_attr_destroy(pthread_attr_t *attr);
extern int pthread_create(pthread_t *thread, const pthread_attr_t *attr, void *(*start)(void *), void *argument);
extern int pthread_join(pthread_t thread, void **result);
extern pthread_t pthread_self(void);
extern int pthread_set_qos_class_self_np(unsigned int class, int relative);
extern int pthread_get_qos_class_np(pthread_t thread, unsigned int *class, int *relative);
extern int pthread_attr_set_qos_class_np(pthread_attr_t *attr, unsigned int class, int relative);
extern int pthread_attr_get_qos_class_np(pthread_attr_t *attr, unsigned int *class, int *relative);
extern void *pthread_override_qos_class_start_np(pthread_t thread, unsigned int class, int relative);
extern int pthread_override_qos_class_end_np(void *override);
extern unsigned int qos_class_self(void);
extern unsigned int qos_class_main(void);

#define EINVAL 22
#define QOS_USER_INTERACTIVE 0x21
#define QOS_USER_INITIATED 0x19
#define QOS_DEFAULT 0x15
#define QOS_UTILITY 0x11
#define QOS_UNSPECIFIED 0

static int failures;
static int step;
static void expect(int actual, int wanted) {
    step = step + 1;
    if (actual != wanted)
        failures = failures + 1 + step * 1000;
}

static unsigned int worker_class;
static int worker_relative;
static unsigned int worker_after;

static void *worker(void *argument) {
    (void)argument;
    pthread_get_qos_class_np(pthread_self(), &worker_class, &worker_relative);
    pthread_set_qos_class_self_np(QOS_UTILITY, -3);
    worker_after = qos_class_self();
    return (void *)0;
}

int main(void) {
    unsigned int class = 99;
    int relative = 99;
    pthread_attr_t attr;
    pthread_t thread;
    void *override;

    /* The main thread starts at user-interactive. */
    expect((int)qos_class_main(), QOS_USER_INTERACTIVE);
    expect((int)qos_class_self(), QOS_USER_INTERACTIVE);
    expect(pthread_get_qos_class_np(pthread_self(), &class, &relative), 0);
    expect((int)class, QOS_USER_INTERACTIVE);
    expect(relative, 0);

    /* Setting a class is visible to this thread only. */
    expect(pthread_set_qos_class_self_np(QOS_USER_INITIATED, -5), 0);
    expect(pthread_get_qos_class_np(pthread_self(), &class, &relative), 0);
    expect((int)class, QOS_USER_INITIATED);
    expect(relative, -5);
    expect((int)qos_class_self(), QOS_USER_INITIATED);

    /* Unknown classes and out-of-range priorities change nothing. */
    expect(pthread_set_qos_class_self_np(QOS_UNSPECIFIED, 0), EINVAL);
    expect(pthread_set_qos_class_self_np(0x42, 0), EINVAL);
    expect(pthread_set_qos_class_self_np(QOS_DEFAULT, 1), EINVAL);
    expect(pthread_set_qos_class_self_np(QOS_DEFAULT, -16), EINVAL);
    expect((int)qos_class_self(), QOS_USER_INITIATED);

    /* A new thread starts at the default class and keeps its own changes. */
    expect(pthread_create(&thread, (const pthread_attr_t *)0, worker, (void *)0), 0);
    expect(pthread_join(thread, (void **)0), 0);
    expect((int)worker_class, QOS_DEFAULT);
    expect(worker_relative, 0);
    expect((int)worker_after, QOS_UTILITY);
    expect((int)qos_class_self(), QOS_USER_INITIATED);

    /* Attributes remember the requested class. */
    expect(pthread_attr_init(&attr), 0);
    expect(pthread_attr_set_qos_class_np(&attr, QOS_UTILITY, -2), 0);
    class = 0;
    relative = 0;
    expect(pthread_attr_get_qos_class_np(&attr, &class, &relative), 0);
    expect((int)class, QOS_UTILITY);
    expect(relative, -2);
    expect(pthread_attr_set_qos_class_np(&attr, 0x42, 0), EINVAL);
    expect(pthread_attr_set_qos_class_np((pthread_attr_t *)0, QOS_UTILITY, 0), EINVAL);
    expect(pthread_attr_destroy(&attr), 0);

    /* Overrides hand back a token that must be ended. */
    override = pthread_override_qos_class_start_np(pthread_self(), QOS_USER_INTERACTIVE, 0);
    expect(override != (void *)0, 1);
    expect(pthread_override_qos_class_end_np(override), 0);
    expect(pthread_override_qos_class_end_np((void *)0), EINVAL);
    expect(pthread_override_qos_class_start_np(pthread_self(), 0x42, 0) == (void *)0, 1);

    if (failures != 0) {
        puts("qos:contract failed");
        return 1;
    }
    puts("qos:classes, priorities, attributes and overrides passed");
    return 0;
}
