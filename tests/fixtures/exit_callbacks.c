extern int puts(const char *);
extern int __cxa_atexit(void (*)(void *),void *,void *);
extern void __cxa_finalize(void *);
extern int atexit(void (*)(void));
static int dso_a,dso_b,count;
static void a(void *arg) { count+=(int)(unsigned long)arg;puts("callback:a"); }
static void b(void *arg) { count+=(int)(unsigned long)arg;puts("callback:b"); }
static void last(void) { puts("callback:last"); }
int main(void) {
    if (__cxa_atexit(a,(void*)1,&dso_a) || __cxa_atexit(b,(void*)2,&dso_b) || __cxa_atexit(a,(void*)4,&dso_a)) return 124;
    __cxa_finalize(&dso_a);__cxa_finalize(&dso_a);
    if (count!=5) return 123;
    if (atexit(last)) return 122;
    puts("callback:main");return 0;
}
