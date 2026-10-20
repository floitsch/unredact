// Copyright (C) 2026 Toit contributors.
// Independent test renderer: Cairo's text API with an exact FreeType font face.
#include <cairo.h>
#include <cairo-ft.h>
#include <ft2build.h>
#include FT_FREETYPE_H
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
  if (argc != 6) {
    fprintf(stderr, "usage: render-cairo FONT TEXT SIZE OUTPUT.png HINT_ON\n");
    return 2;
  }
  FT_Library library;
  FT_Face face;
  if (FT_Init_FreeType(&library) || FT_New_Face(library, argv[1], 0, &face)) return 3;
  cairo_font_face_t *font = cairo_ft_font_face_create_for_ft_face(face, 0);
  cairo_surface_t *scratch = cairo_image_surface_create(CAIRO_FORMAT_RGB24, 1, 1);
  cairo_t *context = cairo_create(scratch);
  cairo_set_font_face(context, font);
  cairo_set_font_size(context, atof(argv[3]));
  cairo_font_options_t *options = cairo_font_options_create();
  cairo_font_options_set_hint_style(options, atoi(argv[5]) ? CAIRO_HINT_STYLE_FULL : CAIRO_HINT_STYLE_NONE);
  cairo_font_options_set_hint_metrics(options, atoi(argv[5]) ? CAIRO_HINT_METRICS_ON : CAIRO_HINT_METRICS_OFF);
  cairo_font_options_set_antialias(options, CAIRO_ANTIALIAS_GRAY);
  cairo_set_font_options(context, options);
  cairo_text_extents_t extents;
  cairo_text_extents(context, argv[2], &extents);
  cairo_surface_t *surface = cairo_image_surface_create(CAIRO_FORMAT_RGB24,
      (int)ceil(extents.width)+28, (int)ceil(extents.height)+28);
  cairo_destroy(context);
  cairo_surface_destroy(scratch);
  context = cairo_create(surface);
  cairo_set_source_rgb(context, 1, 1, 1);
  cairo_paint(context);
  cairo_set_source_rgb(context, 0, 0, 0);
  cairo_set_font_face(context, font);
  cairo_set_font_size(context, atof(argv[3]));
  cairo_set_font_options(context, options);
  cairo_move_to(context, 12.25-extents.x_bearing, 12.25-extents.y_bearing);
  cairo_show_text(context, argv[2]);
  cairo_status_t status = cairo_surface_write_to_png(surface, argv[4]);
  cairo_destroy(context);
  cairo_font_options_destroy(options);
  cairo_surface_destroy(surface);
  cairo_font_face_destroy(font);
  // Cairo may cache scaled font references until its static data is reset.
  cairo_debug_reset_static_data();
  FT_Done_Face(face);
  FT_Done_FreeType(library);
  return status == CAIRO_STATUS_SUCCESS ? 0 : 4;
}
