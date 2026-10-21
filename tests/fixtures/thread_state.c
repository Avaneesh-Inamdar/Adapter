typedef unsigned long pthread_t;
typedef unsigned long pthread_key_t;
typedef struct { long signature; char opaque[8]; } pthread_once_t;
extern int pthread_create(pthread_t *,const void *,void *(*)(void *),void *);
extern int pthread_join(pthread_t,void **);
extern int pthread_key_create(pthread_key_t *,void (*)(void *));
extern int pthread_key_delete(pthread_key_t);
extern void *pthread_getspecific(pthread_key_t);
extern int pthread_setspecific(pthread_key_t,const void *);
extern int pthread_once(pthread_once_t *,void (*)(void));
extern int *__error(void);
extern int puts(const char *);
static pthread_once_t once={0x30b1bcba,{0}};
static pthread_key_t key;
static int initialized;
static int destructor_calls;
static void initialize(void) { initialized++; }
static void destructor(void *arg) {
    __atomic_fetch_add(&destructor_calls,1,__ATOMIC_SEQ_CST);
    if (arg==(void*)1) pthread_setspecific(key,(void*)2);
}
static void *worker(void *arg) {
    if (*__error()!=0 || pthread_getspecific(key)) return (void*)1;
    *__error()=(int)(unsigned long)arg;
    for (int i=0;i<10;i++) if (pthread_once(&once,initialize)) return (void*)2;
    if (initialized!=1 || pthread_setspecific(key,(void*)1) || pthread_getspecific(key)!=(void*)1) return (void*)3;
    if (*__error()!=(int)(unsigned long)arg) return (void*)4;
    return (void*)0;
}
int main(void) {
    if (pthread_key_create(&key,destructor)) return 123;
    *__error()=73;pthread_setspecific(key,(void*)99);
    pthread_t threads[8];
    for (int i=0;i<8;i++) if (pthread_create(&threads[i],(void*)0,worker,(void*)(unsigned long)(i+10))) return 122;
    for (int i=0;i<8;i++) {void *r;if (pthread_join(threads[i],&r) || r) return 121;}
    if (initialized!=1 || destructor_calls!=16 || *__error()!=73 || pthread_getspecific(key)!=(void*)99) return 120;
    if (pthread_key_delete(key) || pthread_key_delete(key)!=22 || pthread_setspecific(key,(void*)1)!=22 || pthread_getspecific(key)) return 119;
    puts("threads:once, keys, destructors and errno passed");return 0;
}
