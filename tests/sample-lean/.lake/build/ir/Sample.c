// Lean compiler output
// Module: Sample
// Imports: public import Init public import Leo4
#include <lean/lean.h>
#if defined(__clang__)
#pragma clang diagnostic ignored "-Wunused-parameter"
#pragma clang diagnostic ignored "-Wunused-label"
#elif defined(__GNUC__) && !defined(__CLANG__)
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wunused-label"
#pragma GCC diagnostic ignored "-Wunused-but-set-variable"
#endif
#ifdef __cplusplus
extern "C" {
#endif
uint64_t lean_uint64_add(uint64_t, uint64_t);
LEAN_EXPORT uint64_t lp_leo4_x2dsample_Sample_add(uint64_t, uint64_t);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_add___boxed(lean_object*, lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_stringify___redArg(lean_object*, lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_stringify(lean_object*, lean_object*, lean_object*);
static const lean_string_object lp_leo4_x2dsample_Sample_hello___closed__0_value = {.m_header = {.m_rc = 0, .m_cs_sz = 0, .m_other = 0, .m_tag = 249}, .m_size = 12, .m_capacity = 12, .m_length = 11, .m_data = "hello, leo4"};
static const lean_object* lp_leo4_x2dsample_Sample_hello___closed__0 = (const lean_object*)&lp_leo4_x2dsample_Sample_hello___closed__0_value;
LEAN_EXPORT const lean_object* lp_leo4_x2dsample_Sample_hello = (const lean_object*)&lp_leo4_x2dsample_Sample_hello___closed__0_value;
lean_object* l_List_lengthTR___redArg(lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen___redArg(lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen___redArg___boxed(lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen(lean_object*, lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen___boxed(lean_object*, lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_maxScalar___redArg(lean_object*, lean_object*, lean_object*);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_maxScalar(lean_object*, lean_object*, lean_object*, lean_object*);
LEAN_EXPORT uint32_t lp_leo4_x2dsample_Sample_constantFortyTwo(lean_object*, uint32_t);
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_constantFortyTwo___boxed(lean_object*, lean_object*);
LEAN_EXPORT uint64_t lp_leo4_x2dsample_Sample_add(uint64_t x_1, uint64_t x_2) {
_start:
{
uint64_t x_3; 
x_3 = lean_uint64_add(x_1, x_2);
return x_3;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_add___boxed(lean_object* x_1, lean_object* x_2) {
_start:
{
uint64_t x_3; uint64_t x_4; uint64_t x_5; lean_object* x_6; 
x_3 = lean_unbox_uint64(x_1);
lean_dec_ref(x_1);
x_4 = lean_unbox_uint64(x_2);
lean_dec_ref(x_2);
x_5 = lp_leo4_x2dsample_Sample_add(x_3, x_4);
x_6 = lean_box_uint64(x_5);
return x_6;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_stringify___redArg(lean_object* x_1, lean_object* x_2) {
_start:
{
lean_object* x_3; 
x_3 = lean_apply_1(x_1, x_2);
return x_3;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_stringify(lean_object* x_1, lean_object* x_2, lean_object* x_3) {
_start:
{
lean_object* x_4; 
x_4 = lean_apply_1(x_2, x_3);
return x_4;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen___redArg(lean_object* x_1) {
_start:
{
lean_object* x_2; 
x_2 = l_List_lengthTR___redArg(x_1);
return x_2;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen___redArg___boxed(lean_object* x_1) {
_start:
{
lean_object* x_2; 
x_2 = lp_leo4_x2dsample_Sample_listLen___redArg(x_1);
lean_dec(x_1);
return x_2;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen(lean_object* x_1, lean_object* x_2) {
_start:
{
lean_object* x_3; 
x_3 = l_List_lengthTR___redArg(x_2);
return x_3;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_listLen___boxed(lean_object* x_1, lean_object* x_2) {
_start:
{
lean_object* x_3; 
x_3 = lp_leo4_x2dsample_Sample_listLen(x_1, x_2);
lean_dec(x_2);
return x_3;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_maxScalar___redArg(lean_object* x_1, lean_object* x_2, lean_object* x_3) {
_start:
{
lean_object* x_4; uint8_t x_5; 
lean_inc(x_3);
lean_inc(x_2);
x_4 = lean_apply_2(x_1, x_2, x_3);
x_5 = lean_unbox(x_4);
if (x_5 == 0)
{
lean_dec(x_2);
return x_3;
}
else
{
lean_dec(x_3);
return x_2;
}
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_maxScalar(lean_object* x_1, lean_object* x_2, lean_object* x_3, lean_object* x_4) {
_start:
{
lean_object* x_5; 
x_5 = lp_leo4_x2dsample_Sample_maxScalar___redArg(x_2, x_3, x_4);
return x_5;
}
}
LEAN_EXPORT uint32_t lp_leo4_x2dsample_Sample_constantFortyTwo(lean_object* x_1, uint32_t x_2) {
_start:
{
uint32_t x_3; 
x_3 = 42;
return x_3;
}
}
LEAN_EXPORT lean_object* lp_leo4_x2dsample_Sample_constantFortyTwo___boxed(lean_object* x_1, lean_object* x_2) {
_start:
{
uint32_t x_3; uint32_t x_4; lean_object* x_5; 
x_3 = lean_unbox_uint32(x_2);
lean_dec(x_2);
x_4 = lp_leo4_x2dsample_Sample_constantFortyTwo(x_1, x_3);
x_5 = lean_box_uint32(x_4);
return x_5;
}
}
lean_object* initialize_Init(uint8_t builtin);
lean_object* initialize_Leo4_Leo4(uint8_t builtin);
static bool _G_initialized = false;
LEAN_EXPORT lean_object* initialize_leo4_x2dsample_Sample(uint8_t builtin) {
lean_object * res;
if (_G_initialized) return lean_io_result_mk_ok(lean_box(0));
_G_initialized = true;
res = initialize_Init(builtin);
if (lean_io_result_is_error(res)) return res;
lean_dec_ref(res);
res = initialize_Leo4_Leo4(builtin);
if (lean_io_result_is_error(res)) return res;
lean_dec_ref(res);
return lean_io_result_mk_ok(lean_box(0));
}
#ifdef __cplusplus
}
#endif
