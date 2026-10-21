typedef unsigned long pthread_t;
extern int pthread_create(pthread_t *,const void *,void *(*)(void *),void *);
extern int pthread_join(pthread_t,void **);
extern pthread_t pthread_self(void);
extern int pthread_equal(pthread_t,pthread_t);
extern int puts(const char *);
extern int library_step(void);
extern __thread int library_count;
static __thread int count=31;
static __thread int zero;
static __thread char buffer[64] __attribute__((aligned(64)));
static pthread_t main_thread;
static void *worker(void *arg) {
    if (count!=31 || zero || ((unsigned long)buffer&63) || library_count!=17) return (void*)111;
    if (pthread_equal(main_thread,pthread_self())) return (void*)112;
    count+=(int)(unsigned long)arg;zero=7;buffer[63]=5;
    if (library_step()!=18 || library_step()!=19 || library_count!=19) return (void*)113;
    if (count!=31+(int)(unsigned long)arg || zero!=7 || buffer[63]!=5) return (void*)114;
    return arg;
}
int main(void) {
    main_thread=pthread_self();
    pthread_t threads[8];
    for (int i=0;i<8;i++) if (pthread_create(&threads[i],(void*)0,worker,(void*)(unsigned long)(i+1))) return 115;
    for (int i=0;i<8;i++) {void *result=(void*)0;if (pthread_join(threads[i],&result) || result!=(void*)(unsigned long)(i+1)) return 116;}
    if (count!=31 || zero || library_count!=17 || library_step()!=18) return 117;
    if (pthread_join(main_thread,(void*)0)!=11 || pthread_join(threads[0],(void*)0)!=3) return 118;
    if (pthread_create(&threads[0],(void*)1,worker,(void*)0)!=22) return 119;
    puts("tls:8 threads and 2 images passed");return 0;
}
