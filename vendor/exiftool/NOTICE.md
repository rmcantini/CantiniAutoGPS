# Bundled ExifTool

This folder contains an unmodified copy of ExifTool 13.55 by Phil Harvey (https://exiftool.org),
with the Perl documentation (`.pod`) files removed and the shebang line set to `#!/usr/bin/perl`.

ExifTool is free software; you can redistribute it and/or modify it under the same terms as
Perl itself (Artistic License or GNU GPL). See the header of `lib/Image/ExifTool.pm`.

CantiniAutoGPS runs it with the system Perl (`/usr/bin/perl`) to write GPS tags into photos.
