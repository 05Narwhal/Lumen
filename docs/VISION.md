# Lumen — Product Vision

## Core Vision

Lumen is a **cross-platform, local-first photo library and professional photo development application** built with Tauri, Rust, and React.

Lumen should not be positioned as simply "a free Lightroom alternative." Its goal is to become a **universal home for a user's photographs**, regardless of operating system, filesystem, storage device, or existing photo-management ecosystem.

### Core principle

> **Your photos. Your files. Your workflow. Everywhere.**

Users should be able to point Lumen at photographs stored anywhere and have Lumen understand and organize them without forcing the user into a proprietary storage system.

Lumen should prioritize:

- Local ownership of photographs
- Cross-platform compatibility
- Filesystem independence
- Non-destructive editing
- Excellent library organization
- Professional RAW development
- Easy migration from existing photo ecosystems
- No cloud service, no account and no subscription: the app is free and open source

---

# Product Positioning

Lumen combines several categories that are normally separate:

1. **Professional photo library / DAM**
2. **RAW photo developer**
3. **Personal photo library**
4. **Photo discovery and memories**
5. **Cross-platform photo migration tool**

Lumen should combine the simplicity of consumer photo applications with the power expected by photographers.

The goal is not to beat every competitor at every individual feature.

The goal is to provide an experience that is difficult to replace because it combines:

- Lightroom-style professional organization
- Professional RAW development
- Apple Photos-style library experience
- Google Photos-style discovery and memories
- Map-based photo exploration
- Cross-platform filesystem support
- Local-first ownership

---

# The Most Important Differentiator: Universal Photo Libraries

Lumen's strongest long-term differentiator should be its ability to work with photographs regardless of where they came from.

Examples include:

- Windows photo folders
- macOS photo libraries
- Linux photo folders
- NTFS drives
- APFS drives
- ext4 drives
- External SSDs/HDDs
- Network storage
- Existing photo archives
- Lightroom catalogs
- Apple Photos libraries
- Standard filesystem folders

The user's originals should not need to be moved into a proprietary Lumen storage format just to use the application.

Whenever technically possible, Lumen should **index and reference existing files rather than unnecessarily copying them**.

---

# Local-First Philosophy

Lumen must work fully without an internet connection for its core local functionality.

A user should be able to:

1. Install Lumen
2. Point it at a photo directory or library
3. Let Lumen index it
4. Browse and organize photographs
5. Edit photographs non-destructively
6. Export photographs

without creating an account or purchasing a subscription.

Never design the core product around the assumption that every user has:

- an account
- an internet connection
- cloud storage
- a subscription

The user should retain ownership and control of their original files.

---

# Universal Import & Migration

Migration should be treated as a **core product feature**, not a secondary utility.

A major target user is someone who wants to leave an existing photo ecosystem.

Lumen should eventually support importing/migrating from systems such as:

- Lightroom Classic catalogs
- Apple Photos libraries
- Standard filesystem photo archives
- XMP sidecars
- Common metadata formats
- Other photo-management applications where practical

When importing an existing library, preserve as much information as technically possible:

- Original file paths
- Metadata
- EXIF
- IPTC
- GPS
- Ratings
- Flags
- Keywords
- Albums/collections
- People
- Faces
- Presets
- Non-destructive edits where possible
- Capture dates
- Orientation
- Color profiles
- Sidecar files

The import system should clearly communicate information that cannot be migrated.

Example:

```text
Lumen Migration

42,817 photographs found

✓ Original files
✓ Ratings
✓ Keywords
✓ GPS
✓ Albums
✓ Metadata
✓ XMP sidecars

⚠ 17 Lightroom-specific adjustments cannot be reproduced

[Import Library]
```

Never silently discard user data.

---

# Lumen Library

The library is the foundation of the application.

Core functionality should include:

- Folders
- Albums
- Smart albums
- Search
- Metadata filtering
- Ratings
- Flags
- Tags/keywords
- Locations
- Map
- People
- Pets
- Memories
- Timeline
- Duplicate detection
- Similar-photo discovery
- Slideshow generation
- Photo/video support where practical

The library should scale toward very large collections.

Design for users with tens or hundreds of thousands of photographs.

Avoid architectures that work well only for small demo libraries.

---

# Map

Location data should be treated as a first-class part of the photo library.

Users should be able to:

- View photographs on a map
- Browse photographs by location
- Group photos by geographic area
- Navigate from map → photos → album/memory
- Use GPS metadata from imported files
- Preserve existing GPS information during migration

The map should eventually connect naturally with Memories.

Example:

```text
Summer 2026
    ↓
Balkans
    ↓
Montenegro → Kosovo → North Macedonia → Greece
    ↓
Photos + route + dates + locations
```

---

# People & Pets

Lumen should support automatic and manual organization around people and pets.

Potential functionality:

- Face detection
- Face clustering
- Manual identity assignment
- People albums
- Pet albums
- Search by person
- Memories involving specific people

Privacy is important.

Whenever technically practical, prefer **on-device processing** for face recognition and personal photo analysis.

Do not require cloud processing for private library features unless the user explicitly chooses it.

---

# Memories

Memories are one of the features that can differentiate Lumen from purely professional DAM software.

Lumen should automatically discover meaningful groups such as:

- Trips
- Years
- Seasons
- Events
- Locations
- People
- Holidays
- Recurring places
- Large collections of photographs taken around the same period

A Memory should be able to combine:

- Photos
- Dates
- Locations
- Map routes
- People
- Captions
- Albums
- Slideshow generation

The goal is to make Lumen useful not only to professional photographers, but also to people who want their entire photographic history to remain meaningful and discoverable.

---

# Develop Mode

Lumen should include a dedicated professional **Develop** workflow.

The initial goal is to cover the core functionality photographers expect from a modern RAW developer.

Potential functionality includes:

- Exposure
- Contrast
- Highlights
- Shadows
- Whites
- Blacks
- Temperature
- Tint
- Saturation
- Vibrance
- Curves
- Color grading
- HSL/color mixer
- Sharpening
- Noise reduction
- Lens corrections
- Chromatic aberration correction
- Transform/perspective
- Crop
- Rotation
- Local adjustments
- Masks
- Presets
- Batch editing
- Advanced export
- RAW-specific processing

Advanced features should be added progressively rather than sacrificing stability for feature count.

---

# AI / On-Device Processing

Whenever practical, AI-powered functionality should run locally.

Examples:

- Denoise
- Subject detection
- Face detection
- Face clustering
- Mask generation
- Sky detection
- Object detection
- Smart categorization

Local processing provides several advantages:

- Privacy
- Offline functionality
- Lower long-term infrastructure costs
- Faster workflows for users with capable hardware
- Stronger local-first positioning

Lumen has no cloud AI service. Anything smart runs on the user's machine.

---

# Presets

Presets should be first-class objects.

Users should be able to:

- Create presets
- Import presets where technically possible
- Export presets
- Apply presets to individual photographs
- Batch-apply presets
- Organize presets
- Share presets

Presets should be portable rather than intentionally locking users into Lumen.

Presets should be able to be read and used from other applications like lightroom preset support where technically possible.

---

# Non-Destructive Editing

Editing should be non-destructive by default.

The original photograph must remain untouched unless the user explicitly performs an operation that modifies the original.

Edits should be represented independently from the source file wherever practical.

The architecture should support:

```text
Original RAW
     │
     ├── Develop settings
     ├── Masks
     ├── Preset
     ├── Metadata
     └── Crop
          │
          ↓
       Rendered result
```

This allows users to change their minds without destroying originals.

---

# Cross-Platform Architecture

Lumen is fundamentally cross-platform.

Primary target platforms:

- Windows
- macOS
- Linux

The architecture should avoid unnecessary platform-specific assumptions.

Filesystem functionality should use abstractions where appropriate rather than assuming:

- Windows drive letters
- Unix paths
- a particular filesystem
- a particular OS directory structure

Platform-specific code is acceptable when required, but it should be isolated behind clear abstractions.

---

# Filesystem Support

A major long-term goal is supporting photo libraries stored on different filesystems from different operating systems.

Examples:

```text
Windows → NTFS
macOS   → APFS
Linux   → ext4
External drives → NTFS/APFS/exFAT/etc.
```

Lumen should distinguish between:

1. The filesystem
2. The operating system
3. The photo file format
4. The Lumen library/index

Do not assume these are the same thing.

A user's Lumen library should be able to reference photographs stored on external or removable volumes.

Handle missing/offline volumes gracefully.

---

# Free and Open

Lumen is free, open-source software under the GPL-3.0. Nothing is paid, gated or held back, and there is no Pro tier and no Cloud service.

- The app is free and community maintained. Anyone may read, build, change and share it under the licence.
- Sync and backup come from the user's own tools (a library folder kept in Syncthing, Nextcloud or OneDrive, say) or from community plugins, never from a Lumen service.
- The only money is optional donations (GitHub Sponsors). Donating unlocks nothing.
- The name and logo are reserved (`TRADEMARKS.md`), so a fork must rename itself.

The product still has to be free enough to spread, powerful enough to replace existing workflows and trustworthy enough to hold someone's entire photo history.

---

# Product-Market Strategy

The primary target audiences are:

## 1. Photographers

Pain points:

- Adobe subscription costs
- Vendor lock-in
- Difficult migration
- Proprietary workflows
- Desire to own their software
- Need for professional RAW development
- Large libraries
- External storage
- Multiple computers/operating systems

Lumen should provide a credible escape route.

---

## 2. Serious Photo-Library Users

Pain points:

- Huge personal photo collections
- Poor organization
- Photos scattered across drives
- Difficulty finding old photographs
- Lack of meaningful memories
- Vendor/platform lock-in
- Need for better search and organization

These users benefit from Lumen even if they never use advanced RAW development.

---

# Competitive Positioning

Do not attempt to win by simply being cheaper.

Free competitors already exist.

Professional competitors already have mature editing pipelines.

Instead, Lumen's competitive advantage should come from the **combination** of:

```text
Professional RAW development
            +
Universal photo library
            +
Cross-platform filesystem support
            +
Migration tools
            +
Local-first architecture
            +
Maps
            +
People/Pets
            +
Memories
            +
Free and open source
```

The key positioning should be:

> **Lumen is not "Lightroom but cheaper."**
>
> **Lumen is a universal, local-first home for your photographs.**

---

# Trust Is a Core Product Feature

Photographers will not trust Lumen with hundreds of thousands of photographs unless the application demonstrates extreme reliability.

Therefore:

- Never silently modify originals
- Never silently delete files
- Never silently move photographs
- Clearly communicate migration limitations
- Provide robust backups before destructive operations
- Make database/index corruption recoverable
- Handle disconnected drives gracefully
- Make library repair/rebuild possible
- Provide clear import/export mechanisms
- Preserve metadata wherever possible
- Prefer transparent file formats
- Avoid unnecessary proprietary lock-in

A photographer should feel that Lumen is a **safe layer over their existing files**, not a box that traps their library.

---

# Technical Product Priorities

When making architectural decisions, prioritize in roughly this order:

1. **Data safety**
2. **Library integrity**
3. **Cross-platform correctness**
4. **Performance with very large libraries**
5. **Non-destructive editing**
6. **Import/migration reliability**
7. **Excellent UX**
8. **Advanced editing features**
9. **Experimental features**

Do not sacrifice library integrity or user data safety for feature velocity.

---

# Development Philosophy

Every feature should be evaluated against:

### Does it make Lumen more useful as a universal photo library?

### Does it reduce switching costs from existing ecosystems?

### Does it strengthen the local-first philosophy?

### Does it work across Windows, macOS, and Linux?

### Does it scale to very large libraries?

### Does it preserve user ownership?

### Does it create unnecessary vendor lock-in?

If a feature does not support these principles, it should require a strong justification.

---

# Long-Term Goal

The long-term goal is for a photographer to be able to say:

> "I have photographs on an old Mac drive, a Windows PC, several external drives, and a NAS. I don't care what filesystem they use or what application created them. I can install Lumen on any computer, point it at my libraries, and have one professional photo environment."

And for an ordinary user:

> "All of my photos from my entire life are in Lumen. I can find them by date, location, person, pet, trip, or memory, and I don't have to give up ownership of the original files."

That is the product Lumen should optimize toward.

---

# Strategic North Star

**Do not build Lumen as a collection of features competing with Lightroom one checkbox at a time.**

Build Lumen around one fundamental promise:

> **Your entire photographic life should be accessible, organized, editable, and portable — regardless of where your files came from or what computer you are using.**

Professional RAW development attracts photographers.

Universal import attracts Lightroom/Photos users.

Maps, people, pets, memories, and discovery attract everyone else.

Local-first architecture builds trust.

Free, open-source software differentiates Lumen from subscription-heavy creative software.

**The product should be free enough to spread, powerful enough to replace existing workflows, trustworthy enough to hold someone's entire photo history, and differentiated enough that users have a reason to choose Lumen instead of simply using another RAW editor.**

-------------------------------

