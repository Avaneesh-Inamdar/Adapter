/* libdispatch block objects and sources: waiting on, cancelling and chaining
   blocks; user-data sources coalesce merged values; timer sources tick, stop on
   cancel and run their registration and cancel handlers. */
typedef void (^block_t)(void);
typedef void *queue_t;

extern int puts(const char *);
extern int usleep(unsigned int);
extern void _Block_release(const void *);

extern struct { char bytes[128]; } _dispatch_source_type_data_add;
extern struct { char bytes[128]; } _dispatch_source_type_data_or;
extern struct { char bytes[128]; } _dispatch_source_type_data_replace;
extern struct { char bytes[128]; } _dispatch_source_type_timer;
extern struct { char bytes[128]; } _dispatch_source_type_read;

extern queue_t dispatch_queue_create(const char *label, void *attr);
extern queue_t dispatch_get_global_queue(long identifier, unsigned long flags);
extern void dispatch_async(queue_t queue, block_t block);
extern void dispatch_sync(queue_t queue, block_t block);
extern void dispatch_release(void *object);
extern void dispatch_resume(void *object);
extern void *dispatch_semaphore_create(long value);
extern long dispatch_semaphore_signal(void *semaphore);
extern long dispatch_semaphore_wait(void *semaphore, unsigned long long timeout);
extern unsigned long long dispatch_time(unsigned long long when, long long delta);

extern block_t dispatch_block_create(unsigned long flags, block_t block);
extern void dispatch_block_perform(unsigned long flags, block_t block);
extern long dispatch_block_wait(block_t block, unsigned long long timeout);
extern void dispatch_block_notify(block_t block, queue_t queue, block_t notification);
extern void dispatch_block_cancel(block_t block);
extern long dispatch_block_testcancel(block_t block);

extern void *dispatch_source_create(void *type, unsigned long handle, unsigned long mask, queue_t queue);
extern void dispatch_source_set_event_handler(void *source, block_t handler);
extern void dispatch_source_set_cancel_handler(void *source, block_t handler);
extern void dispatch_source_set_registration_handler(void *source, block_t handler);
extern void dispatch_source_set_timer(void *source, unsigned long long start, unsigned long long interval, unsigned long long leeway);
extern void dispatch_source_merge_data(void *source, unsigned long value);
extern void dispatch_source_cancel(void *source);
extern long dispatch_source_testcancel(void *source);
extern unsigned long dispatch_source_get_data(void *source);

#define SECOND 1000000000LL
#define ADD(variable, amount) __atomic_fetch_add(&(variable), (amount), __ATOMIC_SEQ_CST)
#define LOAD(variable) __atomic_load_n(&(variable), __ATOMIC_SEQ_CST)

static int failures;
static int step;
static void expect(long actual, long wanted) {
    step = step + 1;
    if (actual != wanted)
        failures = failures + 1 + step * 1000;
}

static void wait_for(void *semaphore) {
    expect(dispatch_semaphore_wait(semaphore, dispatch_time(0, 10 * SECOND)) == 0, 1);
}

static int block_ran;
static int notified;
static int cancelled_ran;
static int perform_ran;
static unsigned long total;
static int events;
static int registrations;
static int cancels;
static unsigned long last_data;
static int ticks;
static int five_tick_signals;
static int one_shot_ticks;

int main(void) {
    queue_t serial = dispatch_queue_create("com.test.source", (void *)0);
    queue_t global = dispatch_get_global_queue(0, 0);
    void *signal = dispatch_semaphore_create(0);

    /* A block object can be waited on, notified about and cancelled. */
    {
        block_t made = dispatch_block_create(0, ^{
            ADD(block_ran, 1);
        });
        dispatch_async(global, made);
        expect(dispatch_block_wait(made, dispatch_time(0, 10 * SECOND)), 0);
        expect(block_ran, 1);
        dispatch_block_notify(made, serial, ^{
            notified = 1;
            dispatch_semaphore_signal(signal);
        });
        wait_for(signal);
        expect(notified, 1);
        _Block_release(made);

        block_t cancelled = dispatch_block_create(0, ^{
            cancelled_ran = 1;
        });
        dispatch_block_cancel(cancelled);
        expect(dispatch_block_testcancel(cancelled) != 0, 1);
        dispatch_sync(serial, cancelled);
        expect(cancelled_ran, 0);
        expect(dispatch_block_wait(cancelled, dispatch_time(0, SECOND)), 0);
        _Block_release(cancelled);

        block_t idle = dispatch_block_create(0, ^{
        });
        expect(dispatch_block_wait(idle, dispatch_time(0, SECOND / 20)) != 0, 1);
        expect(dispatch_block_testcancel(idle), 0);
        _Block_release(idle);

        dispatch_block_perform(0, ^{
            perform_ran = 1;
        });
        expect(perform_ran, 1);
        expect(dispatch_block_create(0x1000, ^{
        }) == (block_t)0, 1);
    }

    /* A DATA_ADD source coalesces merges made before it is resumed. */
    {
        void *source = dispatch_source_create(&_dispatch_source_type_data_add, 0, 0, serial);
        expect(source != (void *)0, 1);
        dispatch_source_set_registration_handler(source, ^{
            ADD(registrations, 1);
        });
        dispatch_source_set_event_handler(source, ^{
            total = total + dispatch_source_get_data(source);
            ADD(events, 1);
            dispatch_semaphore_signal(signal);
        });
        dispatch_source_set_cancel_handler(source, ^{
            ADD(cancels, 1);
            dispatch_semaphore_signal(signal);
        });
        dispatch_source_merge_data(source, 5);
        dispatch_source_merge_data(source, 7);
        usleep(30000);
        expect(LOAD(events), 0);
        dispatch_resume(source);
        wait_for(signal);
        expect((long)total, 12);
        expect(LOAD(events), 1);
        expect(LOAD(registrations), 1);
        dispatch_source_merge_data(source, 3);
        wait_for(signal);
        expect((long)total, 15);

        /* Nothing is delivered after cancellation, and the cancel handler runs. */
        dispatch_source_cancel(source);
        expect(dispatch_source_testcancel(source) != 0, 1);
        wait_for(signal);
        expect(LOAD(cancels), 1);
        dispatch_source_merge_data(source, 9);
        usleep(30000);
        expect(LOAD(events), 2);
        dispatch_release(source);
    }

    /* DATA_OR combines bits and DATA_REPLACE keeps the last non-zero value. */
    {
        void *or_source = dispatch_source_create(&_dispatch_source_type_data_or, 0, 0, serial);
        dispatch_source_set_event_handler(or_source, ^{
            last_data = dispatch_source_get_data(or_source);
            dispatch_semaphore_signal(signal);
        });
        dispatch_source_merge_data(or_source, 1);
        dispatch_source_merge_data(or_source, 4);
        dispatch_resume(or_source);
        wait_for(signal);
        expect((long)last_data, 5);
        dispatch_source_cancel(or_source);
        dispatch_release(or_source);

        void *replace_source = dispatch_source_create(&_dispatch_source_type_data_replace, 0, 0, serial);
        dispatch_source_set_event_handler(replace_source, ^{
            last_data = dispatch_source_get_data(replace_source);
            dispatch_semaphore_signal(signal);
        });
        dispatch_source_merge_data(replace_source, 8);
        dispatch_source_merge_data(replace_source, 0);
        dispatch_source_merge_data(replace_source, 3);
        dispatch_resume(replace_source);
        wait_for(signal);
        expect((long)last_data, 3);
        dispatch_source_cancel(replace_source);
        dispatch_release(replace_source);
    }

    /* A repeating timer ticks, stops when cancelled, and a one-shot fires once. */
    {
        void *timer = dispatch_source_create(&_dispatch_source_type_timer, 0, 0, serial);
        void *five_ticks = dispatch_semaphore_create(0);
        void *timer_cancelled = dispatch_semaphore_create(0);
        dispatch_source_set_timer(timer, dispatch_time(0, SECOND / 50), SECOND / 50, 0);
        dispatch_source_set_event_handler(timer, ^{
            int seen = ADD(ticks, (int)dispatch_source_get_data(timer)) + (int)dispatch_source_get_data(timer);
            if (seen >= 5 && ADD(five_tick_signals, 1) == 0)
                dispatch_semaphore_signal(five_ticks);
        });
        dispatch_source_set_cancel_handler(timer, ^{
            dispatch_semaphore_signal(timer_cancelled);
        });
        dispatch_resume(timer);
        wait_for(five_ticks);
        dispatch_source_cancel(timer);
        wait_for(timer_cancelled);
        {
            int frozen = LOAD(ticks);
            expect(frozen >= 5, 1);
            usleep(120000);
            expect(LOAD(ticks), frozen);
        }
        dispatch_release(timer);

        void *once = dispatch_source_create(&_dispatch_source_type_timer, 0, 0, serial);
        dispatch_source_set_timer(once, dispatch_time(0, SECOND / 50), ~0ULL, 0);
        dispatch_source_set_event_handler(once, ^{
            ADD(one_shot_ticks, (int)dispatch_source_get_data(once));
            dispatch_semaphore_signal(signal);
        });
        dispatch_resume(once);
        wait_for(signal);
        usleep(100000);
        expect(LOAD(one_shot_ticks), 1);
        dispatch_source_cancel(once);
        dispatch_release(once);
    }

    /* Source types that need kernel event sources are refused. */
    expect(dispatch_source_create(&_dispatch_source_type_read, 0, 0, serial) == (void *)0, 1);

    if (failures != 0) {
        puts("dispatch_source:contract failed");
        return 1;
    }
    puts("dispatch_source:block objects, data and timer sources passed");
    return 0;
}
