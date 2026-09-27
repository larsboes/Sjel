# Why Sjel exists

First written 2026-09-27. Each claim carries its source. A claim with no outside source says so.

## The claim

A person's life is spread over many separate apps and services. Each one keeps its own copy of the
data on its own servers, and the person has little say in what happens to it. Sjel is one system
that holds that data on hardware the person controls. It aims to be simple enough for someone
without technical experience, and to carry an assistant that can act on the data ([README.md, What
it does](../README.md#what-it-does)).

## What the sources show

**Daily life runs through many separate apps.** A panel of US smartphone users used about 18
different apps a day in July 2025. Six apps took 49% of the time ([RealityMine,
2025-09-16](https://www.realitymine.com/news/us-consumers-use-18-apps-a-day)). The study counts
apps, not where their data lives.

**People feel they have no control over that data.** 73% of US adults say they have little or no
control over what companies do with data collected about them. Among those who have heard of AI,
70% have little or no trust in companies to decide responsibly how they use it, and 81% of those
familiar with AI expect company use of AI to put their personal information to uses they are not
comfortable with ([Pew Research Center,
2023-10-18](https://www.pewresearch.org/internet/2023/10/18/how-americans-view-data-privacy/),
5,101 US adults).

**In cloud apps the server's copy is the real one.** When the service shuts down, users may be
able to export their data, but they normally cannot keep running the software on it. Kleppmann,
Wiggins, van Hardenberg and McGranaghan set seven ideals against this, among them "the network is
optional" and "you retain ultimate ownership and control" ([Local-first software, Onward!
2019](https://www.inkandswitch.com/essay/local-first/)). Sjel takes those two as requirements: the
node runs without a network, and the data lives on the person's own devices.

**Data that stays on the device has no central point of attack.** Apple builds its assistant on
that principle. When a request needs a larger model, Private Cloud Compute processes it and keeps
nothing after the response ([Apple Security Research,
2024-06-10](https://security.apple.com/blog/private-cloud-compute/)). Sjel's model order follows
the same idea: the device's own model first, then the owner's Mac, and a cloud model only with
pseudonymized data ([README.md, What it does](../README.md#what-it-does)).

## What the sources do not show

- That people want one system for all of this, rather than better separate apps. Unverified.
- That a person without technical experience can run a Sjel node. The first test is a second
  household, not yet measured ([ISA.md](../ISA.md)).
