use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};

use super::{Error, Violation};

const INCLUDE_MACROS: [&str; 3] = ["include", "include_str", "include_bytes"];

/// Where a Rust source sits inside its migration.
#[derive(Debug, Clone, Copy)]
pub(super) enum Layout<'a> {
    /// A single-file migration; it has no private files besides itself.
    File,
    /// A file inside a directory migration.
    Directory {
        /// Module depth below the migration root (`mod.rs` is 0).
        depth: usize,
        /// Path components relative to the migration directory.
        relative: &'a [String],
    },
}

/// Rejects code that depends on anything outside the migration's own files.
///
/// Dependencies on other crates remain allowed: they are explicit Cargo
/// dependencies rather than evolving application modules reached by accident.
pub(super) fn check(logical: &str, source: &str, layout: Layout<'_>) -> Result<(), Error> {
    let file = syn::parse_file(source).map_err(|e| Error::Parse {
        path: logical.to_owned(),
        message: e.to_string(),
    })?;
    let depth = match layout {
        Layout::File => 0,
        Layout::Directory { depth, .. } => depth,
    };
    let mut checker = Checker {
        depth,
        layout,
        violation: None,
    };

    checker.visit_file(&file);

    match checker.violation {
        Some(violation) => Err(Error::Isolation {
            path: logical.to_owned(),
            violation,
        }),
        None => Ok(()),
    }
}

/// Walks one parsed source, keeping the first escape it finds.
struct Checker<'a> {
    depth: usize,
    layout: Layout<'a>,
    violation: Option<Violation>,
}

impl Checker<'_> {
    fn report(&mut self, violation: Violation) {
        self.violation.get_or_insert(violation);
    }

    fn segments<S: AsRef<str>>(&mut self, segments: &[S]) {
        let rendered = || {
            segments
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join("::")
        };
        let supers = segments
            .iter()
            .take_while(|s| s.as_ref() == "super")
            .count();

        if segments.first().is_some_and(|s| s.as_ref() == "crate") {
            self.report(Violation::CratePath(rendered()));
        } else if supers > self.depth {
            self.report(Violation::SuperPath(rendered()));
        }
    }

    fn use_tree(&mut self, tree: &syn::UseTree, prefix: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());

                if path.ident == "super" {
                    self.use_tree(&path.tree, prefix);
                } else {
                    let mut rendered = prefix.clone();

                    first_branch(&path.tree, &mut rendered);
                    self.segments(&rendered);
                }

                prefix.pop();
            }
            syn::UseTree::Name(syn::UseName { ident })
            | syn::UseTree::Rename(syn::UseRename { ident, .. }) => {
                prefix.push(ident.to_string());
                self.segments(prefix);
                prefix.pop();
            }
            syn::UseTree::Glob(_) => self.segments(prefix),
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.use_tree(tree, prefix);
                }
            }
        }
    }

    fn tokens(&mut self, tokens: TokenStream) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        let mut index = 0;

        while index < trees.len() {
            match &trees[index] {
                TokenTree::Ident(ident) if ident == "crate" || ident == "super" => {
                    index = self.token_path(&trees, index);
                }
                TokenTree::Group(group) => {
                    self.tokens(group.stream());
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    /// Checks a `crate`/`super` path inside macro tokens; returns the next unread index.
    fn token_path(&mut self, trees: &[TokenTree], mut index: usize) -> usize {
        let mut segments = Vec::new();
        let mut separated = false;

        while let Some(TokenTree::Ident(ident)) = trees.get(index) {
            segments.push(ident.to_string());

            if !path_separator(trees, index + 1) {
                index += 1;

                break;
            }

            separated = true;
            index += 3;
        }

        if separated {
            self.segments(&segments);
        }

        index
    }

    fn include(&mut self, mac: &syn::Macro) {
        let rendered = mac.tokens.to_string();
        let Ok(literal) = syn::parse2::<syn::LitStr>(mac.tokens.clone()) else {
            return self.report(Violation::Include(rendered));
        };
        let inside = match self.layout {
            Layout::File => false,
            Layout::Directory { relative, .. } => stays_inside(relative, &literal.value()),
        };

        if !inside {
            self.report(Violation::Include(literal.value()));
        }
    }
}

impl<'ast> Visit<'ast> for Checker<'_> {
    fn visit_visibility(&mut self, _: &'ast syn::Visibility) {}

    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if let syn::Meta::List(list) = &attr.meta {
            self.tokens(list.tokens.clone());
        }

        visit::visit_attribute(self, attr);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let idents: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        if path.leading_colon.is_none() {
            self.segments(&idents);
        }

        visit::visit_path(self, path);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if item.leading_colon.is_none() {
            self.use_tree(&item.tree, &mut Vec::new());
        }
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item.attrs.iter().any(|a| a.path().is_ident("path")) {
            self.report(Violation::PathAttribute);
        }

        match (&item.content, self.layout) {
            (None, Layout::File) => self.report(Violation::OutOfLineModule(item.ident.to_string())),
            (None, Layout::Directory { .. }) => {}
            (Some(_), _) => {
                self.depth += 1;
                visit::visit_item_mod(self, item);
                self.depth -= 1;
            }
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let include = mac
            .path
            .segments
            .last()
            .is_some_and(|s| INCLUDE_MACROS.iter().any(|m| s.ident == m));

        if include {
            self.include(mac);
        }

        self.tokens(mac.tokens.clone());
        visit::visit_macro(self, mac);
    }
}

/// Appends the first branch of `tree`, so diagnostics show a concrete path.
fn first_branch(tree: &syn::UseTree, rendered: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(path) => {
            rendered.push(path.ident.to_string());
            first_branch(&path.tree, rendered);
        }
        syn::UseTree::Name(syn::UseName { ident })
        | syn::UseTree::Rename(syn::UseRename { ident, .. }) => rendered.push(ident.to_string()),
        syn::UseTree::Glob(_) => rendered.push("*".into()),
        syn::UseTree::Group(group) => {
            if let Some(tree) = group.items.first() {
                first_branch(tree, rendered);
            }
        }
    }
}

fn path_separator(trees: &[TokenTree], index: usize) -> bool {
    let colon = |i: usize| matches!(trees.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ':');

    colon(index) && colon(index + 1)
}

/// Whether an `include*!` path, relative to the including file, stays within the
/// migration's hashed files; dotfiles are never hashed, so they count as outside.
fn stays_inside(relative: &[String], include: &str) -> bool {
    let mut stack: Vec<&str> = relative[..relative.len() - 1]
        .iter()
        .map(String::as_str)
        .collect();

    if include.starts_with('/') || include.contains('\\') || include.contains(':') {
        return false;
    }

    for component in include.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            name if name.starts_with('.') => return false,
            name => stack.push(name),
        }
    }

    true
}
