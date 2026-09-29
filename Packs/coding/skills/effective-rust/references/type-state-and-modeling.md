# Ergonomic Type-System Modeling & Zero-Cost Abstractions

Rust's expressive type system allows enforcing operational invariants at compile time with zero runtime overhead.

## 1. Type-State Pattern (Make Invalid States Unrepresentable)
Encode finite state machines into types so that operations are only callable in valid lifecycle states:

```rust
pub struct Draft;
pub struct Confirmed;
pub struct Dispatched;

pub struct Order<State> {
    id: OrderId,
    items: Vec<Item>,
    _state: std::marker::PhantomData<State>,
}

impl Order<Draft> {
    pub fn new(id: OrderId) -> Self {
        Self { id, items: Vec::new(), _state: std::marker::PhantomData }
    }

    pub fn add_item(&mut self, item: Item) {
        self.items.push(item);
    }

    pub fn confirm(self) -> Result<Order<Confirmed>, OrderError> {
        if self.items.is_empty() {
            return Err(OrderError::Empty);
        }
        Ok(Order { id: self.id, items: self.items, _state: std::marker::PhantomData })
    }
}

impl Order<Confirmed> {
    // Only confirmed orders can be dispatched!
    pub fn dispatch(self) -> Order<Dispatched> {
        Order { id: self.id, items: self.items, _state: std::marker::PhantomData }
    }
}
```

## 2. Parse, Don't Validate (Newtypes)
Avoid "stringly-typed" architectures. Wrapping raw primitives into dedicated Newtypes prevents unit confusion, cross-entity mixups, and invalid inputs:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct StationId(String);

impl StationId {
    pub fn parse(raw: impl Into<String>) -> Result<Self, InvalidStationError> {
        let s = raw.into();
        if s.starts_with("station:") && s.len() > 8 {
            Ok(Self(s))
        } else {
            Err(InvalidStationError(s))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

## 3. Zero-Copy Borrowing with `Cow<'a, T>` and Serde
* When data typically needs reading without changes, but occasionally requires mutation (e.g. normalization, stripping whitespace, redaction), use `std::borrow::Cow<'a, str>`:
  ```rust
  pub fn sanitize_slug(input: &str) -> std::borrow::Cow<'_, str> {
      if input.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
          std::borrow::Cow::Borrowed(input)
      } else {
          std::borrow::Cow::Owned(input.to_ascii_lowercase().replace(' ', "-"))
      }
  }
  ```
* For zero-copy JSON/binary deserialization where the payload outlives the parsed object, borrow string slices directly from the input buffer:
  ```rust
  #[derive(serde::Deserialize)]
  pub struct IngestPayload<'a> {
      #[serde(borrow)]
      pub topic: &'a str,
      #[serde(borrow)]
      pub body: &'a str,
  }
  ```

## 4. Fine-Grained Error Hierarchies (`thiserror`)
* Capabilities and libraries must declare descriptive, strongly typed errors using `thiserror`.
* Do not swallow errors with blanket wildcards `_ => ()`.
* Map errors explicitly across system boundaries (e.g. database error -> domain error -> HTTP response).
