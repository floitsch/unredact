// Copyright (C) 2026 Toit contributors.
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <math.h>
#include <hb.h>
#include <hb-ot.h>
#include <hb-ft.h>
#include <ft2build.h>
#include FT_FREETYPE_H
#include <cairo.h>
#include <cairo-ft.h>
#include <cairo-pdf.h>
#include <poppler.h>

typedef struct {
    FT_Library library;
    unsigned char *data;
    FT_Face ft;
    FT_Face hinted;
    hb_font_t *hb;
    cairo_font_face_t *cairo;
    int cairo_owns_ft;
} URFont;
typedef struct { FT_Library library; FT_Face face; unsigned char *data; } FTStorage;
static cairo_user_data_key_t ft_storage_key;
static void free_ft_storage(void *data) {
    FTStorage *s=data;
    FT_Done_Face(s->face); FT_Done_FreeType(s->library); free(s->data); free(s);
}
typedef struct { double advance, left, top, right, bottom; int glyphs, missing; } URMetrics;
typedef struct { uint32_t codepoint; double x1,y1,x2,y2,size; char font[128]; } URChar;

void ur_font_close(URFont *f) {
    if (!f) return;
    // Release the measurement face before Cairo can release the shared library.
    if (f->hinted) FT_Done_Face(f->hinted);
    if (f->hb) hb_font_destroy(f->hb);
    if (f->cairo) cairo_font_face_destroy(f->cairo);
    // Cairo's scaled-font cache can retain the face after this wrapper is dropped.
    if (!f->cairo_owns_ft) {
        if (f->ft) FT_Done_Face(f->ft);
        if (f->library) FT_Done_FreeType(f->library);
        free(f->data);
    }
    free(f);
}
URFont *ur_font_open(const unsigned char *data, unsigned int length) {
    if (!length || length > (1U<<30)) return NULL;
    URFont *f = calloc(1, sizeof(*f));
    if (!f) return NULL;
    f->data=malloc(length);
    if (!f->data) { ur_font_close(f); return NULL; }
    memcpy(f->data,data,length);
    if (FT_Init_FreeType(&f->library) || FT_New_Memory_Face(f->library, f->data, length, 0, &f->ft)) { ur_font_close(f); return NULL; }
    // Separate sizes and immutable shared bytes keep Cairo and measurement independent.
    if (FT_New_Memory_Face(f->library, f->data, length, 0, &f->hinted)) { ur_font_close(f); return NULL; }
    hb_blob_t *blob = hb_blob_create((const char *)f->data,length,HB_MEMORY_MODE_READONLY,NULL,NULL);
    hb_face_t *face = hb_face_create(blob, 0);
    f->hb = hb_font_create(face);
    hb_ot_font_set_funcs(f->hb);
    hb_font_set_scale(f->hb, 100000, 100000);
    hb_face_destroy(face); hb_blob_destroy(blob);
    f->cairo = cairo_ft_font_face_create_for_ft_face(f->ft, 0);
    if (cairo_font_face_status(f->cairo)) { ur_font_close(f); return NULL; }
    FTStorage *storage=malloc(sizeof(*storage));
    if(!storage) {ur_font_close(f);return NULL;}
    storage->library=f->library;storage->face=f->ft;storage->data=f->data;
    if(cairo_font_face_set_user_data(f->cairo,&ft_storage_key,storage,free_ft_storage)) {free(storage);ur_font_close(f);return NULL;}
    f->cairo_owns_ft=1;
    return f;
}
static hb_buffer_t *shape_font(hb_font_t *font, const char *text, int kern) {
    hb_buffer_t *b = hb_buffer_create();
    hb_buffer_add_utf8(b,text,-1,0,-1);
    hb_buffer_guess_segment_properties(b);
    hb_feature_t feature = {HB_TAG('k','e','r','n'), kern != 0, 0, (unsigned)-1};
    hb_shape(font,b,&feature,1);
    return b;
}
static hb_buffer_t *shape(URFont *f, const char *text, int kern) {
    return shape_font(f->hb,text,kern);
}
static int measure_font(hb_font_t *font, const char *text, int kern, double units, URMetrics *m) {
    hb_buffer_t *b = shape_font(font,text,kern);
    unsigned count=0;
    hb_glyph_info_t *info=hb_buffer_get_glyph_infos(b,&count);
    hb_glyph_position_t *pos=hb_buffer_get_glyph_positions(b,&count);
    memset(m,0,sizeof(*m));
    double x=0,y=0; int ink=0;
    for(unsigned i=0;i<count;i++) {
        hb_glyph_extents_t e;
        if (!info[i].codepoint) m->missing++;
        if(hb_font_get_glyph_extents(font,info[i].codepoint,&e) && (e.width || e.height)) {
            double l=x+(pos[i].x_offset+e.x_bearing)/units;
            double r=l+e.width/units;
            double t=-(y+(pos[i].y_offset+e.y_bearing)/units);
            double d=t-e.height/units;
            if(!ink) {m->left=l;m->right=r;m->top=t;m->bottom=d;ink=1;}
            else {m->left=fmin(m->left,l);m->right=fmax(m->right,r);m->top=fmin(m->top,t);m->bottom=fmax(m->bottom,d);}
        }
        x+=pos[i].x_advance/units; y+=pos[i].y_advance/units;
    }
    m->advance=x;m->glyphs=(int)count;
    hb_buffer_destroy(b);return 1;
}
int ur_measure(URFont *f, const char *text, int kern, URMetrics *m) {
    return measure_font(f->hb,text,kern,100000.0,m);
}
int ur_measure_hinted(URFont *f, const char *text, int kern, int ppem, URMetrics *m) {
    if (ppem < 6 || ppem > 256 || FT_Set_Pixel_Sizes(f->hinted,ppem,ppem)) return 0;
    hb_font_t *font=hb_ft_font_create_referenced(f->hinted);
    // hb-ft defaults to NO_HINTING; explicitly use FreeType's normal hinted metrics.
    hb_ft_font_set_load_flags(font,FT_LOAD_DEFAULT | FT_LOAD_TARGET_NORMAL);
    int ok=measure_font(font,text,kern,64.0*ppem,m);
    hb_font_destroy(font);
    return ok;
}
static void draw(URFont *f,cairo_t *cr,const char *text,double size,int kern,double tracking,double x,double y) {
    hb_buffer_t *b=shape(f,text,kern);unsigned count=0;
    hb_glyph_info_t *info=hb_buffer_get_glyph_infos(b,&count);
    hb_glyph_position_t *pos=hb_buffer_get_glyph_positions(b,&count);
    cairo_glyph_t *glyphs=cairo_glyph_allocate((int)count);
    double px=x,py=y;
    for(unsigned i=0;i<count;i++) {
        glyphs[i].index=info[i].codepoint;
        glyphs[i].x=px+pos[i].x_offset/100000.0*size;
        glyphs[i].y=py-pos[i].y_offset/100000.0*size;
        px+=pos[i].x_advance/100000.0*size+tracking;
        py-=pos[i].y_advance/100000.0*size;
    }
    cairo_set_font_face(cr,f->cairo);cairo_set_font_size(cr,size);
    cairo_font_options_t *options=cairo_font_options_create();
    cairo_font_options_set_hint_style(options,CAIRO_HINT_STYLE_NONE);
    cairo_font_options_set_hint_metrics(options,CAIRO_HINT_METRICS_OFF);
    cairo_set_font_options(cr,options);cairo_font_options_destroy(options);
    cairo_set_source_rgb(cr,0,0,0);cairo_show_glyphs(cr,glyphs,(int)count);
    cairo_glyph_free(glyphs);hb_buffer_destroy(b);
}
static int copy_gray(cairo_surface_t *surface,uint8_t *out,int width,int height) {
    cairo_surface_flush(surface);
    if(cairo_surface_status(surface)) return 0;
    unsigned char *data=cairo_image_surface_get_data(surface);int stride=cairo_image_surface_get_stride(surface);
    for(int y=0;y<height;y++) for(int x=0;x<width;x++) {
        uint32_t pixel;memcpy(&pixel,data+y*stride+4*x,4);
        out[y*width+x]=(uint8_t)((((pixel>>16)&255)*77+((pixel>>8)&255)*150+(pixel&255)*29)>>8);
    }
    return 1;
}
int ur_render(URFont *f,const char *text,double size,int kern,double tracking,double x,double y,int width,int height,uint8_t *out) {
    cairo_surface_t *s=cairo_image_surface_create(CAIRO_FORMAT_RGB24,width,height);cairo_t *cr=cairo_create(s);
    cairo_set_source_rgb(cr,1,1,1);cairo_paint(cr);draw(f,cr,text,size,kern,tracking,x,y);
    int ok=copy_gray(s,out,width,height);cairo_destroy(cr);cairo_surface_destroy(s);return ok;
}
int ur_render_vertical(URFont *f,const char *text,double size,int kern,double tracking,double x,double y,double vertical_scale,int width,int height,uint8_t *out) {
    cairo_surface_t *s=cairo_image_surface_create(CAIRO_FORMAT_RGB24,width,height);cairo_t *cr=cairo_create(s);
    cairo_set_source_rgb(cr,1,1,1);cairo_paint(cr);
    cairo_translate(cr,0,y);cairo_scale(cr,1,vertical_scale);
    draw(f,cr,text,size,kern,tracking,x,0);
    int ok=copy_gray(s,out,width,height);cairo_destroy(cr);cairo_surface_destroy(s);return ok;
}
void *ur_pdf_open(const char *path,char *error,int capacity) {
    GError *err=NULL;char *uri=g_filename_to_uri(path,NULL,&err);
    PopplerDocument *doc=NULL;
    if(uri) {doc=poppler_document_new_from_file(uri,NULL,&err);g_free(uri);}
    if(err) {g_strlcpy(error,err->message,capacity);g_error_free(err);}
    return doc;
}
void ur_pdf_close(void *doc) {g_object_unref(doc);}
int ur_pdf_pages(void *doc) {return poppler_document_get_n_pages(doc);}
int ur_pdf_size(void *doc,int index,double *width,double *height) {
    PopplerPage *p=poppler_document_get_page(doc,index);if(!p)return 0;
    poppler_page_get_size(p,width,height);g_object_unref(p);return 1;
}
int ur_pdf_render(void *doc,int index,double scale,int width,int height,uint8_t *out) {
    PopplerPage *p=poppler_document_get_page(doc,index);if(!p)return 0;
    cairo_surface_t *s=cairo_image_surface_create(CAIRO_FORMAT_RGB24,width,height);cairo_t *cr=cairo_create(s);
    cairo_set_source_rgb(cr,1,1,1);cairo_paint(cr);cairo_scale(cr,scale,scale);poppler_page_render(p,cr);
    int ok=copy_gray(s,out,width,height);cairo_destroy(cr);cairo_surface_destroy(s);g_object_unref(p);return ok;
}
// Native coordinates are retained even when a large image suggests an OCR overlay.
URChar *ur_pdf_chars(void *doc,int index,int *count,double *image_fraction) {
    *count=0;*image_fraction=0;
    PopplerPage *p=poppler_document_get_page(doc,index);if(!p)return NULL;
    double w,h;poppler_page_get_size(p,&w,&h);
    GList *images=poppler_page_get_image_mapping(p);
    for(GList *l=images;l;l=l->next) {PopplerImageMapping *m=l->data;*image_fraction+=fabs((m->area.x2-m->area.x1)*(m->area.y2-m->area.y1))/(w*h);}
    poppler_page_free_image_mapping(images);
    char *text=poppler_page_get_text(p);PopplerRectangle *rects=NULL;unsigned n=0;
    URChar *out=NULL;
    if(text && poppler_page_get_text_layout(p,&rects,&n) && n) {
        out=calloc(n,sizeof(*out));
        if(out) {
            GList *attrs=poppler_page_get_text_attributes(p);
            const char *cursor=text;unsigned i=0;
            while(*cursor && i<n) {
                out[i].codepoint=g_utf8_get_char(cursor);out[i].x1=rects[i].x1;out[i].y1=rects[i].y1;out[i].x2=rects[i].x2;out[i].y2=rects[i].y2;
                for(GList *l=attrs;l;l=l->next) {PopplerTextAttributes *a=l->data;
                    if((int)i>=a->start_index && (int)i<=a->end_index) {out[i].size=a->font_size;if(a->font_name)g_strlcpy(out[i].font,a->font_name,128);break;}}
                cursor=g_utf8_next_char(cursor);i++;
            }
            *count=(int)i;poppler_page_free_text_attributes(attrs);
        }
    }
    g_free(text);g_free(rects);g_object_unref(p);return out;
}
void ur_free(void *ptr) {free(ptr);}
// Controlled ground-truth PDF generation used by integration tests and benchmarks.
void *ur_pdf_canvas(const char *path,double w,double h) {
    cairo_surface_t *s=cairo_pdf_surface_create(path,w,h);cairo_t *cr=cairo_create(s);cairo_surface_destroy(s);return cr;
}
void ur_pdf_text(void *cr,URFont *f,const char *text,double size,int kern,double tracking,double x,double y) {draw(f,cr,text,size,kern,tracking,x,y);}
void ur_pdf_rect(void *cr,double x,double y,double w,double h) {cairo_set_source_rgb(cr,0,0,0);cairo_rectangle(cr,x,y,w,h);cairo_fill(cr);}
int ur_pdf_finish(void *cr) {cairo_surface_t *s=cairo_get_target(cr);cairo_surface_finish(s);int status=cairo_surface_status(s);cairo_destroy(cr);return status==0;}
