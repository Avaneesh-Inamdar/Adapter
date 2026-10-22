/* libdispatch consumer: queues, ordering, barriers, groups, once, after, apply,
   target queues, suspension, labels and the main queue, driven by real blocks. */
typedef void (^block_t)(void);
typedef void (^index_block_t)(unsigned long);
typedef void *queue_t;

extern int puts(const char *);
extern void exit(int);
extern int usleep(unsigned int);
extern int strcmp(const char *, const char *);

extern struct { char bytes[128]; } _dispatch_main_q;
extern struct { char bytes[128]; } _dispatch_queue_attr_concurrent;
extern queue_t dispatch_queue_create(const char *label, void *attr);
extern queue_t dispatch_queue_create_with_target(const char *label, void *attr, queue_t target);
extern queue_t dispatch_get_global_queue(long identifier, unsigned long flags);
extern const char *dispatch_queue_get_label(queue_t queue);
extern queue_t dispatch_get_current_queue(void);
extern void dispatch_async(queue_t queue, block_t block);
extern void dispatch_sync(queue_t queue, block_t block);
extern void dispatch_barrier_async(queue_t queue, block_t block);
extern void dispatch_barrier_sync(queue_t queue, block_t block);
extern void dispatch_after(unsigned long long when, queue_t queue, block_t block);
extern void dispatch_apply(unsigned long count, queue_t queue, index_block_t block);
extern void dispatch_once(long *predicate, block_t block);
extern void *dispatch_group_create(void);
extern void dispatch_group_async(void *group, queue_t queue, block_t block);
extern void dispatch_group_enter(void *group);
extern void dispatch_group_leave(void *group);
extern long dispatch_group_wait(void *group, unsigned long long timeout);
extern void dispatch_group_notify(void *group, queue_t queue, block_t block);
extern void dispatch_suspend(void *object);
extern void dispatch_resume(void *object);
extern void dispatch_retain(void *object);
extern void dispatch_release(void *object);
extern void dispatch_main(void);
extern void *dispatch_queue_attr_make_initially_inactive(void *attr);
extern void dispatch_activate(void *object);
extern void dispatch_set_context(void *object, void *context);
extern void *dispatch_get_context(void *object);
extern void dispatch_set_finalizer_f(void *object, void (*finalizer)(void *));
extern void dispatch_queue_set_specific(queue_t queue, const void *key, void *context, void (*destructor)(void *));
extern void *dispatch_queue_get_specific(queue_t queue, const void *key);
extern void *dispatch_get_specific(const void *key);
extern void dispatch_async_and_wait(queue_t queue, block_t block);
extern unsigned long long dispatch_walltime(const void *when, long long delta);
extern unsigned int dispatch_queue_get_qos_class(queue_t queue, int *relative);
extern void *dispatch_semaphore_create(long value);
extern long dispatch_semaphore_signal(void *semaphore);
extern long dispatch_semaphore_wait(void *semaphore, unsigned long long timeout);
extern unsigned long long dispatch_time(unsigned long long when, long long delta);

#define MAIN_Q ((queue_t)&_dispatch_main_q)
#define CONCURRENT ((void *)&_dispatch_queue_attr_concurrent)
#define SECOND 1000000000LL
#define ADD(variable) __atomic_fetch_add(&(variable), 1, __ATOMIC_SEQ_CST)

static int failures;
static int step;
static void expect(int actual, int wanted) {
    step = step + 1;
    if (actual != wanted)
        failures = failures + 1 + step * 1000;
}

static int order[100];
static int order_next;
static int readers;
static int barrier_saw;
static int writers_after;
static int writers_saw;
static int notified;
static int once_count;
static long once_token;
static int after_order[2];
static int after_next;
static int suspended_ran;
static int apply_slots[64];
static int target_inside;
static int target_overlap;
static int on_main;
static int main_sync_ran;
static queue_t current_seen;
static int inactive_ran;
static int and_wait_ran;
static long specific_seen;
static int finalized;

static void finalizer(void *context) {
    finalized = (int)(long)context;
}

static void wait_for(void *semaphore) {
    expect(dispatch_semaphore_wait(semaphore, dispatch_time(0, 10 * SECOND)) == 0, 1);
}

int main(void) {
    queue_t serial = dispatch_queue_create("com.test.serial", (void *)0);
    queue_t concurrent = dispatch_queue_create("com.test.concurrent", CONCURRENT);
    queue_t global = dispatch_get_global_queue(0, 0);
    int i;

    /* Labels, root queues and rejected arguments. */
    expect(strcmp(dispatch_queue_get_label(serial), "com.test.serial"), 0);
    expect(strcmp(dispatch_queue_get_label(MAIN_Q), "com.apple.main-thread"), 0);
    expect(strcmp(dispatch_queue_get_label(global), "com.apple.root.default-qos"), 0);
    expect(dispatch_get_global_queue(0, 1) == (queue_t)0, 1);
    expect(dispatch_get_global_queue(12345, 0) == (queue_t)0, 1);
    expect(dispatch_get_global_queue(0, 0) == global, 1);

    /* A serial queue runs submissions one at a time in submission order. */
    for (i = 0; i < 100; i++) {
        int value = i;
        dispatch_async(serial, ^{
            order[order_next] = value;
            order_next = order_next + 1;
        });
    }
    dispatch_sync(serial, ^{
    });
    expect(order_next, 100);
    for (i = 0; i < 100; i++)
        expect(order[i], i);

    /* dispatch_sync runs to completion before it returns and sees the queue. */
    dispatch_sync(serial, ^{
        current_seen = dispatch_get_current_queue();
    });
    expect(current_seen == serial, 1);

    /* A concurrent queue really runs two blocks at once: each waits for the other. */
    {
        void *to_a = dispatch_semaphore_create(0);
        void *to_b = dispatch_semaphore_create(0);
        void *done = dispatch_semaphore_create(0);
        dispatch_async(concurrent, ^{
            dispatch_semaphore_signal(to_b);
            wait_for(to_a);
            dispatch_semaphore_signal(done);
        });
        dispatch_async(concurrent, ^{
            dispatch_semaphore_signal(to_a);
            wait_for(to_b);
            dispatch_semaphore_signal(done);
        });
        wait_for(done);
        wait_for(done);
    }

    /* A barrier waits for earlier blocks and holds back later ones. */
    {
        void *group = dispatch_group_create();
        for (i = 0; i < 4; i++)
            dispatch_group_async(group, concurrent, ^{
                usleep(5000);
                ADD(readers);
            });
        dispatch_barrier_async(concurrent, ^{
            barrier_saw = readers;
        });
        for (i = 0; i < 4; i++)
            dispatch_group_async(group, concurrent, ^{
                ADD(writers_after);
                writers_saw = writers_saw + (barrier_saw == 4);
            });
        expect(dispatch_group_wait(group, dispatch_time(0, 10 * SECOND)), 0);
        dispatch_barrier_sync(concurrent, ^{
        });
        expect(barrier_saw, 4);
        expect(writers_after, 4);
        expect(writers_saw, 4);
        dispatch_release(group);
    }

    /* Groups: wait times out while a member is outstanding, notify fires after. */
    {
        void *group = dispatch_group_create();
        void *finished = dispatch_semaphore_create(0);
        dispatch_group_enter(group);
        expect(dispatch_group_wait(group, dispatch_time(0, SECOND / 20)) != 0, 1);
        dispatch_group_notify(group, serial, ^{
            notified = notified + 1;
            dispatch_semaphore_signal(finished);
        });
        usleep(20000);
        expect(notified, 0);
        dispatch_group_leave(group);
        wait_for(finished);
        expect(notified, 1);
        expect(dispatch_group_wait(group, 0), 0);
        dispatch_release(group);
    }

    /* dispatch_once runs exactly once, also under contention. */
    {
        void *group = dispatch_group_create();
        for (i = 0; i < 8; i++)
            dispatch_group_async(group, concurrent, ^{
                dispatch_once(&once_token, ^{
                    usleep(10000);
                    ADD(once_count);
                });
            });
        dispatch_group_wait(group, dispatch_time(0, 10 * SECOND));
        dispatch_once(&once_token, ^{
            ADD(once_count);
        });
        expect(once_count, 1);
        dispatch_release(group);
    }

    /* dispatch_after fires later than work submitted now. */
    {
        void *done = dispatch_semaphore_create(0);
        dispatch_after(dispatch_time(0, SECOND / 10), global, ^{
            after_order[after_next] = 2;
            after_next = after_next + 1;
            dispatch_semaphore_signal(done);
        });
        dispatch_async(global, ^{
            after_order[after_next] = 1;
            after_next = after_next + 1;
        });
        wait_for(done);
        expect(after_order[0], 1);
        expect(after_order[1], 2);
    }

    /* dispatch_apply covers every index once and returns when all are done. */
    dispatch_apply(64, global, ^(unsigned long index) {
        apply_slots[index] = apply_slots[index] + 1 + (int)index;
    });
    for (i = 0; i < 64; i++)
        expect(apply_slots[i], 1 + i);

    /* Queues that share a serial target never overlap. */
    {
        queue_t target = dispatch_queue_create("com.test.target", (void *)0);
        queue_t first = dispatch_queue_create_with_target("com.test.first", CONCURRENT, target);
        queue_t second = dispatch_queue_create_with_target("com.test.second", CONCURRENT, target);
        void *group = dispatch_group_create();
        for (i = 0; i < 10; i++) {
            dispatch_group_async(group, first, ^{
                if (ADD(target_inside) != 0)
                    target_overlap = 1;
                usleep(1000);
                __atomic_fetch_sub(&target_inside, 1, __ATOMIC_SEQ_CST);
            });
            dispatch_group_async(group, second, ^{
                if (ADD(target_inside) != 0)
                    target_overlap = 1;
                usleep(1000);
                __atomic_fetch_sub(&target_inside, 1, __ATOMIC_SEQ_CST);
            });
        }
        expect(dispatch_group_wait(group, dispatch_time(0, 20 * SECOND)), 0);
        expect(target_overlap, 0);
        dispatch_release(group);
        dispatch_release(first);
        dispatch_release(second);
        dispatch_release(target);
    }

    /* A suspended queue holds its work until it is resumed. */
    {
        queue_t held = dispatch_queue_create("com.test.held", (void *)0);
        dispatch_suspend(held);
        dispatch_async(held, ^{
            suspended_ran = 1;
        });
        usleep(30000);
        expect(suspended_ran, 0);
        dispatch_resume(held);
        dispatch_sync(held, ^{
        });
        expect(suspended_ran, 1);
        dispatch_release(held);
    }

    /* A __block variable and a captured block both survive being copied to the heap. */
    {
        __block int total = 1;
        void *group = dispatch_group_create();
        block_t inner = ^{
            total = total + 1;
        };
        dispatch_group_async(group, serial, ^{
            total = total + 40;
        });
        dispatch_group_async(group, serial, ^{
            inner();
            inner();
        });
        expect(dispatch_group_wait(group, dispatch_time(0, 10 * SECOND)), 0);
        expect(total, 43);
        dispatch_release(group);
    }

    /* Queue-specific data is found on the queue and while running on it, not elsewhere. */
    {
        static int key;
        dispatch_queue_set_specific(serial, &key, (void *)77, (void *)0);
        expect(dispatch_queue_get_specific(serial, &key) == (void *)77, 1);
        expect(dispatch_get_specific(&key) == (void *)0, 1);
        dispatch_sync(serial, ^{
            specific_seen = (long)dispatch_get_specific(&key);
        });
        expect((int)specific_seen, 77);
        dispatch_queue_set_specific(serial, &key, (void *)0, (void *)0);
        expect(dispatch_queue_get_specific(serial, &key) == (void *)0, 1);
    }

    /* Context and finalizer run when the last reference goes. */
    {
        queue_t ephemeral = dispatch_queue_create("com.test.ephemeral", (void *)0);
        dispatch_set_context(ephemeral, (void *)9);
        expect(dispatch_get_context(ephemeral) == (void *)9, 1);
        dispatch_set_finalizer_f(ephemeral, finalizer);
        dispatch_release(ephemeral);
        for (i = 0; i < 500 && finalized == 0; i++)
            usleep(10000);
        expect(finalized, 9);
    }

    /* An initially inactive queue holds work until it is activated. */
    {
        queue_t inactive = dispatch_queue_create("com.test.inactive",
                                                 dispatch_queue_attr_make_initially_inactive((void *)0));
        dispatch_async(inactive, ^{
            inactive_ran = 1;
        });
        usleep(30000);
        expect(inactive_ran, 0);
        dispatch_activate(inactive);
        dispatch_sync(inactive, ^{
        });
        expect(inactive_ran, 1);
        dispatch_release(inactive);
    }

    /* async_and_wait, wall-clock deadlines and queue QoS. */
    dispatch_async_and_wait(serial, ^{
        and_wait_ran = 1;
    });
    expect(and_wait_ran, 1);
    {
        void *group = dispatch_group_create();
        unsigned long long deadline = dispatch_walltime((void *)0, SECOND / 20);
        int relative = 5;
        expect((int)(deadline >> 63), 1);
        dispatch_group_enter(group);
        expect(dispatch_group_wait(group, deadline) != 0, 1);
        dispatch_group_leave(group);
        expect(dispatch_group_wait(group, dispatch_walltime((void *)0, 0)), 0);
        dispatch_release(group);
        expect((int)dispatch_queue_get_qos_class(global, &relative), 0x15);
        expect(relative, 0);
        expect((int)dispatch_queue_get_qos_class(serial, (int *)0), 0);
    }

    dispatch_release(serial);
    dispatch_release(concurrent);
    if (failures != 0) {
        puts("dispatch:queue contract failed");
        exit(1);
    }

    /* The main queue only runs under dispatch_main, including sync from a worker. */
    dispatch_async(MAIN_Q, ^{
        on_main = 1;
        dispatch_async(global, ^{
            dispatch_sync(MAIN_Q, ^{
                main_sync_ran = on_main + 1;
            });
            dispatch_async(MAIN_Q, ^{
                if (main_sync_ran == 2) {
                    puts("dispatch:queues, groups, barriers, once, after, apply and main passed");
                    exit(0);
                }
                puts("dispatch:main queue contract failed");
                exit(1);
            });
        });
    });
    dispatch_main();
    return 1;
}
