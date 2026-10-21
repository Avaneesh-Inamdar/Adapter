/* Same-artifact Objective-C probes: deterministic JSONL, no addresses or clock.
 * The identical binary runs on Darwin and through Adapter; Adapter's libobjc
 * adapters must answer every probe exactly as the host runtime does.
 */
extern int printf(const char *, ...), snprintf(char *, unsigned long, const char *, ...);
extern int strcmp(const char *, const char *);
extern unsigned long strlen(const char *);

extern signed char sel_isEqual(SEL, SEL), class_isMetaClass(Class);
extern SEL sel_registerName(const char *), sel_getUid(const char *);
extern const char *sel_getName(SEL), *class_getName(Class);
extern Class objc_getClass(const char *), class_getSuperclass(Class);
extern unsigned long class_getInstanceSize(Class);
extern id class_createInstance(Class, unsigned long), object_dispose(id);
extern id objc_retain(id), objc_autorelease(id);
extern void objc_release(id), *objc_autoreleasePoolPush(void), objc_autoreleasePoolPop(void *);

static long load_order, load_count, deallocations;

__attribute__((objc_root_class))
@interface Probe
+ (void)load;
+ (long)scale:(long)value;
- (long)add:(long)a with:(long)b;
- (long)compute:(long)value;
- (void)dealloc;
@end
@implementation Probe
+ (void)load {
    load_order = load_order * 10 + 1;
    load_count++;
}
+ (long)scale:(long)value {
    return value;
}
- (long)add:(long)a with:(long)b {
    return a * 3 + b;
}
- (long)compute:(long)value {
    return value + 7;
}
- (void)dealloc {
    deallocations++;
}
@end

@interface Child : Probe
+ (void)load;
@end
@implementation Child
+ (void)load {
    load_order = load_order * 10 + 2;
    load_count++;
}
- (long)compute:(long)value {
    return [super compute:value] + 1;
}
@end

@interface Probe (Scaling)
+ (void)load;
@end
@implementation Probe (Scaling)
+ (void)load {
    load_order = load_order * 10 + 3;
    load_count++;
}
+ (long)scale:(long)value {
    return value * 5;
}
@end

static unsigned long fnv(const char *text) {
    unsigned long hash = 2166136261UL;
    for (const unsigned char *p = (const unsigned char *)text; *p; p++)
        hash = (hash ^ *p) * 16777619UL & 0xffffffffUL;
    return hash;
}

static const char *families[8] = {"selector", "dispatch", "super", "category", "nil", "classinfo",
                                  "ownership", "loadorder"};

int main(void) {
    Class probe = objc_getClass("Probe"), child = objc_getClass("Child");
    if (!probe || !child)
        return 3;
    for (int index = 0; index < DIFF_CASE_COUNT; index++) {
        unsigned long seed = (0x6d2b79f5UL ^ ((unsigned long)index * 0x9e3779b9UL)) & 0xffffffffUL;
        seed ^= (seed << 13) & 0xffffffffUL;
        seed ^= seed >> 17;
        seed ^= (seed << 5) & 0xffffffffUL;
        long a = (long)(seed % 1001), b = (long)((seed >> 10) % 1001);
        const char *family = families[index % 8];
        long result[4];
        int used = 1;
        switch (index % 8) {
        case 0: {
            char name[32];
            snprintf(name, sizeof name, "dse:%d", index);
            SEL registered = sel_registerName(name), looked = sel_getUid(name);
            result[0] = (long)sel_isEqual(registered, looked);
            result[1] = (long)sel_isEqual(registered, registered);
            result[2] = strcmp(sel_getName(registered), name) == 0;
            result[3] = (long)strlen(sel_getName(registered));
            used = 4;
            break;
        }
        case 1: {
            id object = class_createInstance(probe, 0);
            result[0] = [(Probe *)object add:a with:b];
            object_dispose(object);
            break;
        }
        case 2: {
            id object = class_createInstance(child, 0);
            result[0] = [object compute:a];
            object_dispose(object);
            break;
        }
        case 3:
            result[0] = [probe scale:a];
            break;
        case 4:
            result[0] = [(Probe *)0 add:a with:b];
            result[1] = [(Probe *)0 compute:a];
            used = 2;
            break;
        case 5:
            result[0] = (long)fnv(class_getName(child));
            result[1] = (long)fnv(class_getName(class_getSuperclass(child)));
            result[2] = (long)class_getInstanceSize(probe);
            result[3] = (long)class_isMetaClass(probe);
            used = 4;
            break;
        case 6: {
            id object = class_createInstance(probe, 0);
            id retained = objc_retain(object);
            void *pool = objc_autoreleasePoolPush();
            objc_release(retained);
            objc_autorelease(object);
            objc_autoreleasePoolPop(pool);
            result[0] = deallocations;
            result[1] = retained == object;
            used = 2;
            break;
        }
        case 7:
            result[0] = load_order;
            result[1] = load_count;
            used = 2;
            break;
        }
        printf("{\"family\":\"%s\",\"id\":\"%s:%d\",\"index\":%d,\"result\":[", family, family,
               index, index);
        for (int i = 0; i < used; i++)
            printf(i ? ",%ld" : "%ld", result[i]);
        printf("]}\n");
    }
    return 0;
}
