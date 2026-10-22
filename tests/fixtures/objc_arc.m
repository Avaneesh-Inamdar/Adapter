/* Built twice: MRC root runtime client, then real compiler-generated ARC code. */
typedef signed char Bool;
extern int printf(const char *, ...), puts(const char *);
extern id class_createInstance(Class, unsigned long) __attribute__((ns_returns_retained));
extern id object_dispose(id);
extern unsigned long pthread_self(void);
extern int pthread_create(unsigned long *, const void *, void *(*)(void *), void *);
extern int pthread_join(unsigned long, void **);
extern long arc_deallocs, arc_nodes;

__attribute__((objc_root_class))
@interface ArcRoot {
    Class _isa;
}
+ (id)alloc;
+ (id)new;
+ (id)factory;
- (id)init;
- (long)magic;
- (void)dealloc;
#if !__has_feature(objc_arc)
- (id)retain;
- (oneway void)release;
- (id)autorelease;
- (unsigned long)retainCount;
- (Bool)allowsWeakReference;
- (Bool)retainWeakReference;
#endif
@end

#ifdef ADAPTER_ARC_ROOT
extern id _objc_rootRetain(id), _objc_rootAutorelease(id);
extern void _objc_rootRelease(id);
extern unsigned long _objc_rootRetainCount(id);
extern Bool _objc_rootTryRetain(id), _objc_rootIsDeallocating(id);
extern id objc_retain(id), objc_autorelease(id);
extern void objc_release(id);
extern id objc_initWeak(id *, id), objc_storeWeakOrNil(id *, id), objc_loadWeakRetained(id *);
extern void objc_copyWeak(id *, id *), objc_moveWeak(id *, id *), objc_destroyWeak(id *);
long arc_deallocs, arc_nodes;
static long retains, releases, autoreleases;
static int reentrant, probe_dying;
static long dying_result;
@implementation ArcRoot
+ (id)alloc {
    return class_createInstance(self, 0);
}
+ (id)new {
    return [[self alloc] init];
}
+ (id)factory {
    return [[self new] autorelease];
}
- (id)init {
    return self;
}
- (long)magic {
    return 42;
}
- (id)retain {
    return _objc_rootRetain(self);
}
- (oneway void)release {
    _objc_rootRelease(self);
}
- (id)autorelease {
    return _objc_rootAutorelease(self);
}
- (unsigned long)retainCount {
    return _objc_rootRetainCount(self);
}
- (Bool)allowsWeakReference {
    return !_objc_rootIsDeallocating(self);
}
- (Bool)retainWeakReference {
    return _objc_rootTryRetain(self);
}
- (void)dealloc {
    __atomic_fetch_add(&arc_deallocs, 1, __ATOMIC_SEQ_CST);
    if (probe_dying) {
        id weak = 0;
        dying_result = objc_storeWeakOrNil(&weak, self) == 0 && objc_loadWeakRetained(&weak) == 0;
        objc_destroyWeak(&weak);
    }
    if (reentrant) {
        reentrant = 0;
        [[ArcRoot new] autorelease];
    }
    object_dispose(self);
}
@end
@interface ArcCounting : ArcRoot
@end
@implementation ArcCounting
- (id)retain {
    retains++;
    return _objc_rootRetain(self);
}
- (oneway void)release {
    releases++;
    _objc_rootRelease(self);
}
- (id)autorelease {
    autoreleases++;
    return _objc_rootAutorelease(self);
}
@end
void arc_weakops(long *out) {
    id object = [ArcRoot new], first, copy, moved;
    objc_initWeak(&first, object);
    objc_copyWeak(&copy, &first);
    objc_moveWeak(&moved, &copy);
    out[0] = first == object && moved == object;
    out[1] = copy == 0;
    objc_release(object);
    out[2] = first == 0 && moved == 0;
    objc_destroyWeak(&first);
    objc_destroyWeak(&copy);
    objc_destroyWeak(&moved);
    probe_dying = 1;
    objc_release([ArcRoot new]);
    probe_dying = 0;
    out[3] = dying_result;
}
void arc_schedule_reentrant(void) {
    reentrant = 1;
    [[ArcRoot new] autorelease];
}
void arc_custom_begin(void) {
    retains = releases = autoreleases = 0;
    id object = [ArcCounting new];
    objc_retain(object);
    objc_release(object);
    objc_autorelease(object);
}
void arc_custom_counts(long *out) {
    out[0] = retains;
    out[1] = releases;
    out[2] = autoreleases;
}
void *arc_retained_create(void) { return [ArcRoot new]; }
#else
extern void arc_weakops(long *), arc_schedule_reentrant(void), arc_custom_begin(void),
    arc_custom_counts(long *);
extern void *arc_retained_create(void);
@interface ArcNode : ArcRoot {
    ArcRoot *_child;
    __weak ArcRoot *_observer;
}
@property(nonatomic, strong) ArcRoot *child;
@property(nonatomic, weak) ArcRoot *observer;
@end
@implementation ArcNode
@synthesize child = _child, observer = _observer;
+ (id)factory {
    return [[self alloc] init];
}
- (void)dealloc {
    __atomic_fetch_add(&arc_nodes, 1, __ATOMIC_SEQ_CST);
}
@end
static __weak ArcRoot *thread_weak;
static long bad_weak;
static void *weak_worker(void *unused) {
    (void)unused;
    @autoreleasepool {
        for (int i = 0; i < 64; i++) {
            ArcRoot *held = thread_weak;
            if (held && [held magic] != 42)
                __atomic_store_n(&bad_weak, 1, __ATOMIC_SEQ_CST);
        }
    }
    return 0;
}
static long deaths(void) { return __atomic_load_n(&arc_deallocs, __ATOMIC_SEQ_CST); }
#ifndef DIFF_CASE_COUNT
#define DIFF_CASE_COUNT 8
__attribute__((constructor)) static void arc_constructor(void) { puts("objc:ARC constructor"); }
#endif
static const char *families[8] = {"strong",      "weak",     "weakops", "pool",
                                  "returnvalue", "customrr", "bridge",  "threads"};
int main(void) {
    for (int index = 0; index < DIFF_CASE_COUNT; index++) {
        long out[4] = {0}, before = deaths();
        @autoreleasepool {
            switch (index % 8) {
            case 0: {
                ArcNode *parent = [ArcNode new];
                ArcRoot *child = [ArcRoot new];
                __weak ArcRoot *weak = child;
                parent.child = child;
                parent.observer = child;
                child = 0;
                out[0] = weak != 0 && parent.observer == weak;
                parent.child = 0;
                out[1] = parent.observer == 0 && weak == 0;
                parent = 0;
                break;
            }
            case 1: {
                __weak ArcNode *weak;
                @autoreleasepool {
                    ArcNode *owner __attribute__((objc_precise_lifetime)) = [ArcNode new];
                    weak = owner;
                    out[0] = [weak magic] == 42;
                }
                out[1] = weak == 0;
                break;
            }
            case 2:
                arc_weakops(out);
                break;
            case 3:
                @autoreleasepool {
                    arc_schedule_reentrant();
                }
                out[0] = deaths() - before;
                @autoreleasepool {
                    ArcRoot *value = [ArcRoot factory];
                    out[1] = [value magic];
                }
                break;
            case 4: {
                __weak ArcNode *weak;
                @autoreleasepool {
                    ArcNode *value __attribute__((objc_precise_lifetime)) = [ArcNode factory];
                    weak = value;
                    out[0] = [value magic];
                }
                out[1] = weak == 0;
                @autoreleasepool {
                    (void)[ArcNode factory];
                }
                out[2] = deaths() - before;
                break;
            }
            case 5:
                @autoreleasepool {
                    arc_custom_begin();
                }
                arc_custom_counts(out);
                break;
            case 6: {
                __weak ArcRoot *weak;
                @autoreleasepool {
                    ArcRoot *value = (__bridge_transfer ArcRoot *)arc_retained_create();
                    void *kept = (__bridge_retained void *)value;
                    value = 0;
                    ArcRoot *again __attribute__((objc_precise_lifetime)) =
                        (__bridge_transfer ArcRoot *)kept;
                    weak = again;
                    out[0] = [again magic];
                }
                out[1] = weak == 0;
                break;
            }
            default: {
                ArcRoot *owner = [ArcRoot new];
                thread_weak = owner;
                bad_weak = 0;
                unsigned long worker;
                int error = pthread_create(&worker, 0, weak_worker, 0);
                owner = 0;
                if (!error)
                    error = pthread_join(worker, 0);
                out[0] = error;
                out[1] = __atomic_load_n(&bad_weak, __ATOMIC_SEQ_CST);
                out[2] = thread_weak == 0;
                thread_weak = 0;
                break;
            }
            }
        }
        /* Lifetime counts sampled only after all temporary owners/pools drain. */
        if (index % 8 != 2)
            out[3] = deaths() - before;
        printf("{\"id\":\"%s:%d\",\"family\":\"%s\",\"index\":%d,\"result\":[%ld,%ld,%ld,%ld]}\n",
               families[index % 8], index, families[index % 8], index, out[0], out[1], out[2],
               out[3]);
    }
    return 0;
}
#endif
