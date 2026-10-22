/* dispatch_sync onto the queue the caller is already running on is a client bug. */
typedef void (^block_t)(void);
typedef void *queue_t;

extern int puts(const char *);
extern queue_t dispatch_queue_create(const char *label, void *attr);
extern void dispatch_sync(queue_t queue, block_t block);

int main(void) {
    queue_t queue = dispatch_queue_create("com.test.deadlock", (void *)0);
    puts("dispatch_deadlock:before");
    dispatch_sync(queue, ^{
        dispatch_sync(queue, ^{
            puts("dispatch_deadlock:unreachable");
        });
    });
    return 0;
}
