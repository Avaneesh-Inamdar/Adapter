/* SDK-free category consumer: methods added to and overriding a registered class. */
extern int puts(const char *);
extern id class_createInstance(Class, unsigned long), object_dispose(id);
extern Class objc_getClass(const char *);
static int order;
__attribute__((objc_root_class))
@interface CategoryRoot
+ (void)load;
+ (int)value;
+ (int)tag;
+ (int)extra;
- (int)value;
- (int)extra;
@end
@implementation CategoryRoot
+ (void)load {
    order = order * 10 + 1;
}
+ (int)value {
    return 1;
}
+ (int)tag {
    return 30;
}
+ (int)extra {
    return 100;
}
- (int)value {
    return 10;
}
- (int)extra {
    return 100;
}
@end
@interface CategoryChild : CategoryRoot
+ (void)load;
@end
@implementation CategoryChild
+ (void)load {
    order = order * 10 + 3;
}
@end
@interface CategoryRoot (Extra)
+ (void)load;
+ (int)value;
- (int)value;
@end
@implementation CategoryRoot (Extra)
+ (void)load {
    order = order * 10 + 2;
}
+ (int)value {
    return 2;
}
- (int)value {
    return 20;
}
@end
int main(void) {
    /* Every class +load runs before any category +load. */
    if (order != 132)
        return 1;
    if ([CategoryRoot value] != 2)
        return 2;
    if ([CategoryRoot extra] != 100)
        return 3;
    CategoryRoot *object = class_createInstance(objc_getClass("CategoryRoot"), 0);
    if (!object)
        return 4;
    if ([object value] != 20)
        return 5;
    if ([object extra] != 100)
        return 6;
    object_dispose(object);
    CategoryChild *child = class_createInstance(objc_getClass("CategoryChild"), 0);
    if (!child)
        return 7;
    if ([child value] != 20 || [CategoryChild value] != 2)
        return 8;
    object_dispose(child);
    puts("objc:category methods override and extend the class");
    return 0;
}
