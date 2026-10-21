/* SDK-free, compiler-emitted Apple Objective-C metadata and message sends. */
extern int puts(const char *), strcmp(const char *, const char *);
extern id class_createInstance(Class, unsigned long), object_dispose(id);
extern Class object_getClass(id), class_getSuperclass(Class), objc_getClass(const char *);
extern const char *class_getName(Class), *sel_getName(SEL);
extern unsigned long class_getInstanceSize(Class);
extern int class_isMetaClass(Class), sel_isEqual(SEL, SEL);
extern SEL sel_registerName(const char *);
extern id objc_retain(id), objc_autorelease(id);
extern void objc_release(id), *objc_autoreleasePoolPush(void), objc_autoreleasePoolPop(void *);
static int loads, initializations, deallocated;
__attribute__((constructor)) static void constructor(void) { puts("objc:constructor"); }
#define CHECK(e)                                                                                   \
    do {                                                                                           \
        if (!(e))                                                                                  \
            return __LINE__;                                                                       \
    } while (0)

__attribute__((objc_root_class))
@interface AdapterRoot {
    Class _isa;
    long _value;
}
+ (void)load;
+ (void)initialize;
+ (id)alloc;
+ (id)new;
+ (long)value;
- (id)init;
- (void)dealloc;
- (long)value;
- (long)sum:(long)a b:(long)b c:(long)c d:(long)d e:(long)e f:(long)f g:(long)g h:(long)h;
- (double)mix:(double)a
            b:(double)b
            c:(double)c
            d:(double)d
            e:(double)e
            f:(double)f
            g:(double)g
            h:(double)h
            i:(double)i;
@end
@implementation AdapterRoot
+ (void)load {
    loads = loads * 10 + 1;
}
+ (void)initialize {
    initializations = initializations * 10 + 1;
}
+ (id)alloc {
    return class_createInstance(self, 0);
}
+ (id)new {
    return [[self alloc] init];
}
+ (long)value {
    return 11;
}
- (id)init {
    _value = 7;
    return self;
}
- (void)dealloc {
    deallocated++;
    object_dispose(self);
}
- (long)value {
    return _value;
}
- (long)sum:(long)a b:(long)b c:(long)c d:(long)d e:(long)e f:(long)f g:(long)g h:(long)h {
    return _value + a + b * 2 + c * 3 + d * 4 + e * 5 + f * 6 + g * 7 + h * 8;
}
- (double)mix:(double)a
            b:(double)b
            c:(double)c
            d:(double)d
            e:(double)e
            f:(double)f
            g:(double)g
            h:(double)h
            i:(double)i {
    return a + b * 2 + c * 3 + d * 4 + e * 5 + f * 6 + g * 7 + h * 8 + i * 9;
}
@end
@interface AdapterChild : AdapterRoot
+ (void)load;
+ (void)initialize;
+ (long)value;
- (long)value;
@end
@implementation AdapterChild
+ (void)load {
    loads = loads * 10 + 2;
}
+ (void)initialize {
    initializations = initializations * 10 + 2;
}
+ (long)value {
    return [super value] + 12;
}
- (long)value {
    return [super value] + 5;
}
@end
int main(void) {
    CHECK(loads == 12 && initializations == 0);
    CHECK([AdapterChild value] == 23 && initializations == 12);
    CHECK([AdapterRoot value] == 11 && initializations == 12);
    AdapterChild *object = [AdapterChild new];
    CHECK(object && [object value] == 12);
    CHECK([object sum:1 b:2 c:3 d:4 e:5 f:6 g:7 h:8] == 211);
    CHECK([object mix:1 b:2 c:3 d:4 e:5 f:6 g:7 h:8 i:9] == 285.0);
    AdapterChild *nil = (id)0;
    CHECK([nil value] == 0 && [nil mix:1 b:2 c:3 d:4 e:5 f:6 g:7 h:8 i:9] == 0.0);
    Class child = objc_getClass("AdapterChild"), root = objc_getClass("AdapterRoot");
    CHECK(object_getClass(object) == child && class_getSuperclass(child) == root);
    CHECK(class_isMetaClass(object_getClass((id)child)) && !class_isMetaClass(child));
    CHECK(!strcmp(class_getName(child), "AdapterChild") && class_getInstanceSize(child) >= 16);
    SEL first = sel_registerName("value"), second = sel_registerName("value");
    CHECK(first == second && sel_isEqual(first, second) && !strcmp(sel_getName(first), "value"));
    CHECK(!objc_getClass("NoSuchAdapterClass"));
    CHECK(objc_retain(object) == object);
    objc_release(object);
    CHECK(!deallocated);
    void *pool = objc_autoreleasePoolPush();
    CHECK(objc_autorelease(object) == object);
    objc_autoreleasePoolPop(pool);
    CHECK(deallocated == 1);
    puts("objc:metadata, inheritance, initialize, GP/SSE/stack, nil and ownership passed");
    return 0;
}
