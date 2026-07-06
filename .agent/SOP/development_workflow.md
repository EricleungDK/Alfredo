# Development Workflow

**Last Updated**: 2026-07-06
**For**: developers

## Related Docs
- [Project Architecture](../System/project_architecture.md) - Understand the system first
- [Database Migrations SOP](database_migrations.md) - How to update database schema if needed
---

## Table of Contents
1. [Setup](#setup)
2. [Daily Workflow](#daily-workflow)
3. [Git Workflow](#git-workflow)
4. [Testing Before Commit](#testing-before-commit)
5. [Common Tasks](#common-tasks)

---

## Setup

### First-Time Setup (15 minutes)

```

```

---

## Daily Workflow

### Starting Your Day

```
```

### Before Making Changes

1. **Pull latest changes**:
   ```bash
   git checkout main
   git pull origin main
   ```

2. **Create feature branch**:
   ```bash
   git checkout -b feature/your-feature-name
   ```

3. **Read relevant docs**:
   - `.agent/System/` for architecture
   - `CLAUDE.md` or `AGENTS.md` for coding standards
   - Existing code for patterns

---

## Git Workflow

### Branch Naming Convention

```
feature/add-community-feed      # New features
fix/wardrobe-load-bug           # Bug fixes
refactor/ai-provider-cleanup    # Code refactoring
docs/update-architecture        # Documentation
test/add-outfit-tests           # Tests only
chore/update-dependencies       # Maintenance
```

### Commit Message Format

Follow Conventional Commits:

```
type(scope): subject

body (optional)

footer (optional)
```

**Types**:
- `feat`: New feature
- `fix`: Bug fix
- `refactor`: Code refactoring
- `docs`: Documentation changes
- `test`: Adding/updating tests
- `chore`: Maintenance tasks
- `perf`: Performance improvements
- `style`: Code style changes (formatting, etc.)

**Examples**:
```bash
feat(wardrobe): add filter by category
fix(camera): resolve image upload timeout
refactor(ai): extract AI provider to factory pattern
docs(database): update schema documentation
test(outfit): add unit tests for outfit generation
chore(deps): update expo to 54.0.13
```

### Commit Workflow

```bash
# 1. Stage changes
git add .

# 2. Review what you're committing
git status
git diff --staged

# 3. Commit with message
git commit -m "feat(community): add post creation feature"

# 4. Push to remote
git push origin feature/your-feature-name
```

### Pull Request Workflow

1. **Push your branch**:
   ```bash
   git push origin feature/your-feature-name
   ```

2. **Create PR on GitHub/GitLab**:
   - Title: Clear, descriptive (e.g., "Add community feed feature")
   - Description: What changed, why, testing steps
   - Link related issues

3. **PR Template**:
   ```markdown
   ## Summary
   Brief description of changes

   ## Changes
   - Added community feed screen
   - Created get-feed Edge Function
   - Updated database with posts table

   ## Testing
   - [ ] Tested on iOS simulator
   - [ ] Tested on Android emulator
   - [ ] Ran unit tests
   - [ ] Tested Edge Function locally

   ## Screenshots/Videos
   [Attach visuals if UI changes]

   ## Related Issues
   Closes #123
   ```

4. **Request Review**:
   - Assign at least 1 reviewer
   - Respond to feedback promptly
   - Make requested changes

5. **Merge**:
   - Squash commits if many small commits
   - Use "Merge commit" for feature branches
   - Delete branch after merge

---

## Testing Before Commit

### Pre-Commit Checklist

**ALWAYS run before committing**:

```bash
# 1. Type check

# 2. Lint (if configured)

# 3. Format check (if using Prettier)

# 4. Run tests

# 5. Build check (optional but recommended)

```

### Manual Testing Checklist


---

## Common Tasks

---

## Debugging

### 

```

### 

# Check terminal output
```

### Database Issues

```

---

## Code Style

### TypeScript

- **No `any` types** - Always use proper types
- **Explicit return types** on functions
- **Interfaces over types** for objects
- **Consistent naming**:
  - PascalCase for components, types, interfaces
  - camelCase for variables, functions
  - UPPER_CASE for constants

### React Native

- **Functional components** only
- **Hooks** for state management
- **Custom hooks** for reusable logic
- **Components** in `components/` directory
- **Screens** in `app/` directory

### Comments

- **Only when necessary** - code should be self-documenting
- **Explain "why" not "what"**
- **TODOs with context**:
  ```typescript
  // TODO: Optimize this query once we have >1000 items
  // TODO (username): Discuss this approach with team
  ```

---

**Document Owner**: Engineering Team
**Review Frequency**: Monthly or when workflow changes
