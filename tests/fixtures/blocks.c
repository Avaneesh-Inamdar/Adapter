/* Block runtime consumer: a copied Clang block must preserve its captured
   value and invoke function after the defining frame is still alive. */
typedef unsigned long size_t;
extern void *_Block_copy(const void *);
extern void _Block_release(const void *);
extern int printf(const char *, ...), puts(const char *);

int main(void) {
    int captured = 37;
    void (^handler)(int) = ^(int value) {
        printf("blocks:value=%d\n", value + captured);
    };
    void *copy = _Block_copy(handler);
    printf("blocks:distinct=%u\n", copy != (void *)handler);
    ((void (^)(int))copy)(5);
    _Block_release(copy);
    puts("blocks:passed");
    return 0;
}
